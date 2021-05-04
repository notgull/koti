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
use std::{future::Future, path::Path, pin::Pin};

const THREAD_COUNT: usize = 4;

const FRAME_SOURCES: &[fn(
    &'static Context,
) -> Pin<Box<dyn Future<Output = crate::Result> + Send + 'static>>] = &[|cx| {
    Box::pin(async move {
        process::process(
            reddit_text_source::reddit_text_source("AskReddit", 10000, 1000, 100, "day", cx)
                .await?,
        )
        .await
    })
}];

static CONTEXT: Context = Context::new();

#[inline]
async fn entry(homedir: &Path) -> crate::Result {
    // create the context
    let ctx = context::Context::default();

    // create a base dir to use
    let basedirname: String = tls_rng().generate::<usize>().to_string();
    let basedir = homedir.join(basedirname);

    // create the directory
    tokio::fs::create_dir_all(&basedir).await;

    CONTEXT.set_basedir(basedir).await;

    // create a guard that deletes the base directory on exit
    struct DeleteTheBasedirOnExit;

    impl Drop for DeleteTheBasedirOnExit {
        #[inline]
        fn drop(&mut self) {
            tokio::runtime::Handle::current()
                .spawn(async { tokio::fs::remove_dir_all(&CONTEXT.basedir().await).await });
        }
    }

    let _guard = DeleteTheBasedirOnExit;

    // select a random element from the array
    let frame_source = FRAME_SOURCES[tls_rng().generate_range::<usize>(0, FRAME_SOURCES.len() - 1)];

    // spawn two tasks: one for creating the thumbnail and one for creating the video proper
    let t1 = tokio::spawn(async move { frame_source(&CONTEXT).await });
    let t2 = tokio::spawn(thumbnail::create_thumbnail(&CONTEXT));

    let (t1, t2) = futures_lite::future::zip(t1, t2).await;
    t1??;
    t2??;

    // now that we have a video and a thumbnail, upload to YouTube
    youtube::upload_to_youtube(&CONTEXT).await
}

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
                match tokio::spawn(entry(&path)).await {
                    Ok(Ok(())) => break,
                    Err(e) => {
                        log::error!("Tokio error: {:?}", e);
                        break;
                    }
                    Ok(Err(e)) => {
                        log::error!("A fatal error occurred: {:?}", e);
                        CONTEXT.reset().await;
                        break;
                    }
                }
            }
        });
}
