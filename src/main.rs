/*
 * This file is part of KOTI.
 *
 * KOTI is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * KOTI is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Afero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with KOTI.  If not, see <https://www.gnu.org/licenses/>.
 */

pub mod context;
mod error;
pub mod filter;
pub mod frame;
pub mod image_size;
pub mod mlt;
pub mod music;
mod process;
mod reddit_text_source;
mod scp;
pub mod text2image;
mod thumbnail;
pub mod util;
mod youtube;

pub use error::*;
pub use frame::*;

use clap::{App, Arg, SubCommand};
use context::Context;
use futures_lite::stream::{self, Stream, StreamExt};
use nanorand::{tls_rng, RNG};
use std::{
    env,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::exit,
    str::FromStr,
    sync::Arc,
};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

const THREAD_COUNT: usize = 4;

macro_rules! reddit_text_source {
    ($sub: expr, $ut: expr, $ct: expr, $rt: expr, $net: expr) => {{
        |cx| {
            async fn inner(
                sub: &str,
                ut: i64,
                ct: i64,
                rt: i64,
                net: &str,
                cx: Arc<Context>,
            ) -> crate::Result {
                let f = reddit_text_source::reddit_text_source(sub, ut, ct, rt, net, &cx).await?;
                println!("Created reddit text stream source");

                process::process(f, cx).await
            }

            Box::pin(inner($sub, $ut, $ct, $rt, $net, cx))
        }
    }};
}

const FRAME_SOURCES: &[fn(
    Arc<Context>,
) -> Pin<Box<dyn Future<Output = crate::Result> + Send + 'static>>] = &[
    reddit_text_source!("AskReddit", 500, 200, 100, "day"),
    reddit_text_source!("TalesFromTechSupport", 500, 100, 100, "week"),
    reddit_text_source!("StoresAboutKevin", 250, 50, 50, "week"),
    reddit_text_source!("NoSleep", 750, 200, 100, "week"),
];

#[inline]
async fn create_video(homedir: PathBuf, datadir: PathBuf, upload: bool) -> crate::Result {
    // create the context
    let ctx = Arc::new(context::Context::default());

    // create a base dir to use
    let basedirname = tls_rng().generate::<usize>();
    let basedir = homedir.join(format!("koti{}", basedirname));
    log::info!("Setting up shop at {:?}", &basedir);

    // create the directory
    tokio::fs::create_dir_all(&basedir).await?;

    ctx.set_basedir(basedir).await;
    ctx.set_datadir(datadir).await;

    // create a guard that deletes the base directory on exit
    //    struct DeleteTheBasedirOnExit(Arc<Context>);

    //    let _guard = DeleteTheBasedirOnExit(ctx.clone());

    // select a random element from the array
    let frame_source = FRAME_SOURCES[tls_rng().generate_range::<usize>(0, FRAME_SOURCES.len())];

    // spawn two tasks: one for creating the thumbnail and one for creating the video proper
    let ctx_clone = ctx.clone();
    let ctx_clone2 = ctx.clone();
    let t1 = tokio::spawn(async move { frame_source(ctx_clone).await });
    let t2 = tokio::spawn(async move {
        let ctx = ctx_clone2;
        thumbnail::create_thumbnail(ctx).await
    });

    let (t1, t2) = futures_lite::future::zip(t1, t2).await;
    t1??;
    t2??;

    // now that we have a video and a thumbnail, upload to YouTube
    if upload {
        youtube::upload_to_youtube(&ctx).await?
    } else {
        let viddir = dirs::video_dir().unwrap();
        let vidpath = viddir.join(format!("koti{}.webm", basedirname));
        tokio::fs::rename(ctx.take_video_path().await, &vidpath).await?;
        log::info!("Moved video to {:?}", &vidpath);
        let thumbpath = viddir.join(format!("koti{}.png", basedirname));
        tokio::fs::rename(ctx.take_thumbnail_path().await, &thumbpath).await?;
        log::info!("Moved thumbnail to {:?}", &thumbpath);
    }

    tokio::fs::remove_dir_all(ctx.basedir().await).await?;

    Ok(())
}

#[inline]
async fn add_music_track(datadir: PathBuf, name: String, musicpath: PathBuf) -> crate::Result {
    let ctx = context::Context::default();
    tokio::fs::create_dir_all(&datadir).await?;
    ctx.set_datadir(datadir).await;

    let mut cout = io::stdout();
    let mut cin = io::stdin();
    let mut attribution = String::new();

    cout.write_all(b"Write the attribution for the music below:\n")
        .await?;
    cin.read_to_string(&mut attribution).await?;

    let mut m = music::Music::load(&ctx).await?;
    m.add_track(name, musicpath, attribution);
    m.save(&ctx).await?;

    cout.write_all(b"Saved!\n").await?;

    Ok(())
}

#[inline]
async fn add_thumbnail(
    datadir: PathBuf,
    id: String,
    tpath: PathBuf,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> crate::Result {
    let ctx = context::Context::default();
    tokio::fs::create_dir_all(&datadir).await?;
    ctx.set_datadir(datadir).await;

    thumbnail::add_thumbnail_to_collection(&ctx, id, tpath, x, y, w, h).await?;

    println!("New thumbnail saved!");

    Ok(())
}

#[inline]
async fn draw_text_image(txt: String, path: PathBuf) -> crate::Result {
    let (img, _, _) =
        text2image::text_overlay(&txt, 24.0, 800, 600, [255, 255, 255], [0, 0, 0], 4).await?;
    tokio::task::spawn_blocking(move || img.save(path)).await??;
    Ok(())
}

fn main() {
    // sets up the logging framework
    env_logger::init();

    // get the home directory and KOTI data directory
    let path = dirs::home_dir().unwrap_or_else(|| Path::new("/").to_path_buf());
    let default_datadir = {
        let p: Option<PathBuf> = env::var_os("KOTI_DATA").map(|e| e.into());
        p
    }
    .unwrap_or_else(|| match dirs::data_dir() {
        Some(mut d) => {
            d.push("koti");
            d
        }
        None => Path::new("/koti").to_path_buf(),
    });

    // configure the app
    let matches = App::new("King of the Internet")
        .version("0.1.0")
        .author("notgull <jtnunley01@gmail.com>")
        .about("Automatically aggregates internet content")
        .arg(
            Arg::with_name("datadir")
                .short("d")
                .long("datadir")
                .value_name("FILE")
                .help("Sets the directory that contains KOTI's information")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("no-upload")
                .long("no-upload")
                .takes_value(false)
                .help("Upload to youtube?"),
        )
        .subcommand(
            SubCommand::with_name("music")
                .about("adds or removes music tracks to be selected in video")
                .subcommand(
                    SubCommand::with_name("add")
                        .about("adds a music track")
                        .arg(
                            Arg::with_name("trackname")
                                .index(1)
                                .required(true)
                                .help("Name of the track")
                                .value_name("TRACK_NAME")
                                .required(true),
                        )
                        .arg(
                            Arg::with_name("trackpath")
                                .index(2)
                                .required(true)
                                .help("Path to the track"),
                        ),
                ),
        )
        .subcommand(
            SubCommand::with_name("thumbnail")
                .about("adds or removes thumbnail templates")
                .arg(
                    Arg::with_name("id")
                        .index(1)
                        .required(true)
                        .value_name("THUMBNAIL_ID"),
                )
                .arg(
                    Arg::with_name("path")
                        .index(2)
                        .required(true)
                        .value_name("BASE_IMAGE_PATH"),
                )
                .arg(Arg::with_name("x").index(3).required(true).value_name("X"))
                .arg(Arg::with_name("y").index(4).required(true).value_name("Y"))
                .arg(
                    Arg::with_name("w")
                        .index(5)
                        .required(true)
                        .value_name("WIDTH"),
                )
                .arg(
                    Arg::with_name("h")
                        .index(6)
                        .required(true)
                        .value_name("HEIGHT"),
                ),
        )
        .subcommand(
            SubCommand::with_name("imagetext")
                .about("debug feature to debug image text")
                .arg(
                    Arg::with_name("path")
                        .index(1)
                        .value_name("PATH")
                        .required(true),
                )
                .arg(
                    Arg::with_name("text")
                        .index(2)
                        .value_name("TEXT")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("ytoken")
                .about("Set the YouTube API token")
                .arg(
                    Arg::with_name("id")
                        .index(1)
                        .value_name("CLIENT_ID")
                        .required(true),
                )
                .arg(
                    Arg::with_name("secret")
                        .index(2)
                        .value_name("CLIENT_SECRET")
                        .required(true),
                ),
        )
        .subcommands({
            if cfg!(debug_assertions) {
                vec![SubCommand::with_name("ytupload")
                    .arg(
                        Arg::with_name("vidpath")
                            .index(1)
                            .value_name("VIDPATH")
                            .required(true),
                    )
                    .arg(
                        Arg::with_name("thumbpath")
                            .index(2)
                            .value_name("THUMBPATH")
                            .required(true),
                    )]
            } else {
                vec![]
            }
        })
        .get_matches();

    let datadir: PathBuf = match matches.value_of_os("datadir") {
        Some(datadir) => datadir.into(),
        None => default_datadir,
    };

    // start the tokio multi-threaded runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Unable to construct Tokio runtime")
        .block_on(async move {
            let local = tokio::task::LocalSet::new();

            // add a music track if need be
            if let Some(matches) = matches.subcommand_matches("music") {
                if let Some(matches) = matches.subcommand_matches("add") {
                    let trackname = matches.value_of("trackname").unwrap().to_string();
                    let trackpath: PathBuf = matches.value_of_os("trackpath").unwrap().into();
                    match tokio::spawn(add_music_track(datadir, trackname, trackpath)).await {
                        Ok(Ok(())) => (),
                        Err(e) => log::error!("A panick occurred: {:?}", e),
                        Ok(Err(e)) => log::error!("Unable to save music track: {:?}", e),
                    }

                    return;
                }
            } else if let Some(matches) = matches.subcommand_matches("imagetext") {
                let path: PathBuf = matches.value_of_os("path").unwrap().into();
                let text = matches.value_of("text").unwrap().to_string();
                match tokio::spawn(draw_text_image(text, path)).await {
                    Ok(Ok(())) => (),
                    Err(e) => log::error!("Panicked: {:?}", e),
                    Ok(Err(e)) => log::error!("Unable to draw image: {:?}", e),
                }

                return;
            } else if let Some(matches) = matches.subcommand_matches("thumbnail") {
                let id = matches.value_of("id").unwrap().to_string();
                let path: PathBuf = matches.value_of_os("path").unwrap().into();
                let x = u32::from_str(matches.value_of("x").unwrap()).expect("X isn't a number");
                let y = u32::from_str(matches.value_of("y").unwrap()).expect("Y isn't a number");
                let w =
                    u32::from_str(matches.value_of("w").unwrap()).expect("Width isn't a number");
                let h =
                    u32::from_str(matches.value_of("h").unwrap()).expect("Height isn't a number");

                match tokio::spawn(add_thumbnail(datadir, id, path, x, y, w, h)).await {
                    Ok(Ok(())) => (),
                    Err(e) => log::error!("Panicked: {:?}", e),
                    Ok(Err(e)) => log::error!("Unable to add thumbnail: {:?}", e),
                }

                return;
            } else if let Some(matches) = matches.subcommand_matches("ytoken") {
                let id = matches.value_of("id").unwrap().to_string();
                let secret = matches.value_of("secret").unwrap().to_string();
                let mut ctx = context::Context::default();
                tokio::fs::create_dir_all(&datadir)
                    .await
                    .expect("Cant make dir");
                ctx.set_datadir(datadir).await;

                match tokio::spawn(async move {
                    let ctx = ctx;
                    youtube::set_token(&ctx, id, secret).await
                })
                .await
                {
                    Ok(Ok(())) => (),
                    Err(e) => log::error!("Panicked: {:?}", e),
                    Ok(Err(e)) => log::error!("Unable to set token: {:?}", e),
                }

                return;
            } else if let Some(matches) = matches.subcommand_matches("ytupload") {
                let vidpath: PathBuf = matches.value_of_os("vidpath").unwrap().into();
                let thumbpath: PathBuf = matches.value_of_os("thumbpath").unwrap().into();
                let ctx = context::Context::default();
                tokio::fs::create_dir_all(&datadir)
                    .await
                    .expect("Can't create dirs?");
                ctx.set_datadir(datadir).await;

                local
                    .run_until(async move {
                        let ctx = Box::leak::<'static>(Box::new(ctx));

                        match tokio::task::spawn_local(youtube::upload_video(
                            ctx,
                            vidpath,
                            thumbpath,
                            "Test".to_string(),
                            "Test".to_string(),
                        ))
                        .await
                        {
                            Ok(Ok(())) => (),
                            Err(e) => log::error!("Panicked: {:?}", e),
                            Ok(Err(e)) => log::error!("Unable to upload video: {:?}", e),
                        }
                    })
                    .await;

                return;
            }

            // try to create a video
            local
                .run_until(async move {
                    for i in 0..10 {
                        match tokio::task::spawn_local(create_video(
                            path.clone(),
                            datadir.clone(),
                            !matches.is_present("no-upload"),
                        ))
                        .await
                        {
                            Ok(Ok(())) => break,
                            Err(e) => {
                                log::error!("Panick error: {:?}", e);
                                break;
                            }
                            Ok(Err(e)) => {
                                log::error!("A fatal error occurred: {:?}", e);
                                match i {
                                    i @ 9 => {
                                        log::error!(
                                            "Tried to run program {} times and failed; stopping...",
                                            i
                                        );
                                        exit(1);
                                    }
                                    i => log::error!("Retrying for the {}nth time", i + 1),
                                }
                            }
                        }
                    }
                })
                .await;
        });
}
