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
pub mod frame;
pub mod image_size;
pub mod mlt;
pub mod music;
mod process;
mod reddit_text_source;
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
) -> Pin<Box<dyn Future<Output = crate::Result> + Send + 'static>>] =
    &[reddit_text_source!("AskReddit", 500, 200, 100, "day")];

#[inline]
async fn create_video(homedir: PathBuf, datadir: PathBuf) -> crate::Result {
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
    struct DeleteTheBasedirOnExit(Arc<Context>);

    impl Drop for DeleteTheBasedirOnExit {
        #[inline]
        fn drop(&mut self) {
            let ctx = self.0.clone();
            //tokio::spawn(async move { tokio::fs::remove_dir_all(ctx.basedir().await).await });
        }
    }

    let _guard = DeleteTheBasedirOnExit(ctx.clone());

    // select a random element from the array
    let frame_source = FRAME_SOURCES[tls_rng().generate_range::<usize>(0, FRAME_SOURCES.len())];

    // spawn two tasks: one for creating the thumbnail and one for creating the video proper
    let ctx_clone = ctx.clone();
    let ctx_clone2 = ctx.clone();
    let t1 = tokio::spawn(async move { frame_source(ctx_clone).await });
    let t2 = tokio::spawn(async move {
        let ctx = ctx_clone2;
        thumbnail::create_thumbnail(&ctx).await
    });

    let (t1, t2) = futures_lite::future::zip(t1, t2).await;
    t1??;
    t2??;

    // now that we have a video and a thumbnail, upload to YouTube
    youtube::upload_to_youtube(&ctx).await
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
            }

            // try to create a video
            loop {
                match tokio::spawn(create_video(path, datadir)).await {
                    Ok(Ok(())) => break,
                    Err(e) => {
                        log::error!("Panick error: {:?}", e);
                        break;
                    }
                    Ok(Err(e)) => {
                        log::error!("A fatal error occurred: {:?}", e);
                        break;
                    }
                }
            }
        });
}
