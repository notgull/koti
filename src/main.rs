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
mod process;
mod reddit_text_source;
mod thumbnail;
pub mod util;
mod youtube;

pub use error::*;
pub use frame::*;

use context::Context;
use futures_lite::stream::{self, Stream, StreamExt};
use nanorand::{tls_rng, RNG};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

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
                let f = reddit_text_source::reddit_text_source(sub, ut, ct, rt, net, &cx)
                    .await?;
                println!("Created reddit text stream source");

                process::process(f).await
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
async fn entry(homedir: PathBuf) -> crate::Result {
    // create the context
    let ctx = Arc::new(context::Context::default());

    // create a base dir to use
    let basedirname = tls_rng().generate::<usize>();
    let basedir = homedir.join(format!("koti{}", basedirname));
    log::info!("Setting up shop at {:?}", &basedir);

    // create the directory
    tokio::fs::create_dir_all(&basedir).await?;

    ctx.set_basedir(basedir).await;

    // create a guard that deletes the base directory on exit
    struct DeleteTheBasedirOnExit(Arc<Context>);

    impl Drop for DeleteTheBasedirOnExit {
        #[inline]
        fn drop(&mut self) {
            let ctx = self.0.clone();
            tokio::spawn(async move { tokio::fs::remove_dir_all(ctx.basedir().await).await });
        }
    }

    let _guard = DeleteTheBasedirOnExit(ctx.clone());

    // select a random element from the array
    let frame_source = FRAME_SOURCES[tls_rng().generate_range::<usize>(0, FRAME_SOURCES.len())];

    // spawn two tasks: one for creating the thumbnail and one for creating the video proper
    let ctx_clone = ctx.clone();
    /*let ctx_clone2 = ctx.clone();
    let t1 = tokio::spawn(async move { frame_source(ctx_clone).await });
    let t2 = tokio::spawn(async move {
        let ctx = ctx_clone2;
        thumbnail::create_thumbnail(&ctx).await
    });

    let (t1, t2) = futures_lite::future::zip(t1, t2).await;
    t1??;
    t2??;*/
    frame_source(ctx_clone).await?;

    // now that we have a video and a thumbnail, upload to YouTube
    youtube::upload_to_youtube(&ctx).await
}

/*#[inline]
async fn entry(homedir: PathBuf) -> crate::Result {
    let cx = Context::default();
    reddit_text_source::reddit_text_source("AskReddit", 1000, 200, 100, "day", &cx)
                    .await?.for_each(|f| log::info!("{:?}", f)).await;

    Ok(())
}*/

fn main() {
    // sets up the logging framework
    env_logger::init();

    // get the home directory
    let path = dirs::home_dir().unwrap_or_else(|| Path::new("/").to_path_buf());

    // start the tokio multi-threaded runtime
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Unable to construct Tokio runtime")
        .block_on(async move {
            loop {
                match tokio::spawn(entry(path)).await {
                    Ok(Ok(())) => break,
                    Err(e) => {
                        log::error!("Tokio error: {:?}", e);
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
