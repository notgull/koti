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

use crate::{
    context::Context,
    mlt::{Filter, PlaylistEntry, Transition},
    util::{video_length, ImmediateOrTask, MapFuture},
    Frame,
};
use futures_lite::{
    future,
    stream::{self, Stream, StreamExt},
};
use once_cell::sync::Lazy;
use quick_xml::{
    events::{BytesDecl, BytesEnd, BytesStart, Event},
    Writer,
};
use regex::Regex;
use std::{
    array::IntoIter as ArrayIter, io::BufWriter, iter, mem, os::unix::ffi::OsStrExt, path::Path,
    process::Stdio, str::FromStr, sync::Arc,
};
use tokio::{
    fs::{self, File},
    process::Command,
};

mod frame;
pub mod tts;

pub const FPS: f32 = 29.97;

#[inline]
fn seconds_to_frames(s: f32) -> usize {
    (s * FPS) as usize
}

macro_rules! s {
    ($e: expr) => {{
        ($e).as_ref()
    }};
}

#[inline]
pub async fn process<S: Stream<Item = Frame>>(frames: S, ctx: Arc<Context>) -> crate::Result {
    let basedir = ctx.basedir().await;
    let datadir = ctx.datadir().await;

    // launch an alternate task to choose a piece of music
    let ctx_clone = ctx.clone();
    let musictask = tokio::spawn(async move {
        let music = crate::music::Music::load(&ctx_clone).await?;
        let (path, attr) = music.random_track();
        let total = video_length(path).await?;

        crate::Result::Ok((path.to_path_buf(), attr.to_string(), total))
    });

    // collect all of the frames into a vector of tasks that are turning those frames into real stuff
    let ctx_clone = ctx.clone();
    let frames_tasks: Vec<_> = frames
        .map(move |f| tokio::spawn(frame::ConvertedFrame::from_frame(f, ctx_clone.clone())))
        .collect()
        .await;

    // collect all of the converted frames and add them to the melt command
    let mut duration: usize = 0;
    let frames = stream::iter(frames_tasks.into_iter())
        .then(std::convert::identity)
        .map(|e| match e {
            Ok(e) => e,
            Err(e) => Err(crate::Error::Join(e)),
        });

    // configure melt to use these frames
    let (video_width, video_height) = ctx.video_size();
    let mut mlt = crate::mlt::Mlt::new(&basedir, video_width, video_height);

    let blacktrack = mlt.add_producer(Path::new("black").to_path_buf());

    // get the intro track, if we have it
    let intro_path = datadir.join("intro.mkv");
    let intro_frame = match fs::metadata(&intro_path).await {
        Err(_) => { log::error!("Did not find an intro frame"); None },
        Ok(_) => {
            let total = (video_length(&intro_path).await? * FPS) as usize;
            let intro_producer = mlt.add_producer(intro_path);
            Some((intro_producer, total))
        }
    };

    // get the outro track, if we have it
    let outro_path = datadir.join("outro.mkv");
    let outro_frame = match fs::metadata(&outro_path).await {
        Err(_) => { log::error!("Did not find an outro frame"); None },
        Ok(_) => {
            let total = (video_length(&outro_path).await? * FPS) as usize;
            let outro_producer = mlt.add_producer(outro_path);
            Some((outro_producer, total))
        }
    };

    // map each frame into an mlt action
    log::info!("Resolving all of the converted frames from their tasks...");
    let frame_tractors: Vec<(String, usize)> =
        stream::iter(intro_frame.into_iter().map(Result::Ok))
            .chain(frames.map(|frame| match frame {
                Ok(frame) => match frame.into_tractor(&mut mlt, &ctx) {
                    Ok((tractor, dur)) => {
                        duration += dur;
                        Ok((tractor, dur))
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }))
            .chain(stream::iter(outro_frame.into_iter().map(Result::Ok)))
            .try_collect()
            .await?;

    // combine the tractors into a single playlist
    let frame_playlist = mlt
        .add_playlist(
            frame_tractors
                .into_iter()
                .map(|(tractor, dur)| PlaylistEntry::Producer {
                    id: tractor,
                    start: 0,
                    end: dur,
                }),
            iter::empty(),
        )
        .to_string();

    // if the total duration is less than a minute, something is wrong
    if duration < seconds_to_frames(60.0) {
        return Err(crate::Error::StaticMsg("duration is less than a minute"));
    }

    // by now, we should be done choosing a music entry
    let (musicpath, attr, musicdur) = musictask.await??;
    let musicdur = seconds_to_frames(musicdur);
    ctx.append_to_description(format!("Music Credits:\n{}\n", attr))
        .await;

    // create a producer for the music and a playlist that plays the music over and over until we reach the total duration
    let music_producer = mlt.add_producer(musicpath).to_string();
    let music_playlist = mlt
        .add_playlist(
            iter::repeat(music_producer).scan(duration, |duration, prod| {
                let len = match duration.checked_sub(musicdur) {
                    Some(newdur) => {
                        *duration = newdur;
                        musicdur
                    }
                    None if *duration == 0 => {
                        return None;
                    }
                    None => {
                        let d = mem::replace(duration, 0);
                        d
                    }
                };
                Some(PlaylistEntry::Producer {
                    id: prod,
                    start: 0,
                    end: len,
                })
            }),
            iter::once(
                Filter::new("volume".to_string()).property("max_gain".to_string(), {
                    // change to taste
                    "-10dB".to_string()
                }),
            ),
        )
        .to_string();

    // use both as tracks
    let main_tractor = mlt
        .add_tractor_with_transitions(
            ArrayIter::new([/*blacktrack,*/ music_playlist, frame_playlist]),
            iter::empty(),
            iter::once(Transition::new("mix", "0", "1", 0, duration)),
        )
        .to_string();

    // run mlt
    let outvideo = mlt.run(main_tractor, duration).await?;

    // register the outvideo in the context and return
    ctx.set_video_path(outvideo).await;

    Ok(())
}
