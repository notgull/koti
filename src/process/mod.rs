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
    util::{ImmediateOrTask, MapFuture},
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
use tokio::{fs::File, process::Command};

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

    // launch an alternate task to choose a piece of music
    let ctx_clone = ctx.clone();
    let musictask = tokio::spawn(async move {
        static DURATION_REGEX: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"Duration: (\d\d):(\d\d):(\d\d).(\d\d)").expect("Regex failed to compile")
        });

        let music = crate::music::Music::load(&ctx_clone).await?;
        let (path, attr) = music.random_track();

        // figure out the length of the sound file using ffmpeg
        let mut c = Command::new("ffmpeg")
            .arg("-i")
            .arg(path)
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()
            .await?;

        // it is supposed to fail

        let mut textout =
            String::from_utf8(mem::take(&mut c.stdout)).expect("ffmpeg output isn't utf-8?");
        textout.extend(iter::once(
            String::from_utf8(c.stderr).expect("ffmpeg stderr isn't utf-8?"),
        ));
        let total: f32 = match DURATION_REGEX.captures(&textout) {
            Some(caps) => caps
                .iter()
                .skip(1)
                .map(|cap| match cap {
                    Some(cap) => usize::from_str(cap.as_str()).expect("Not really an f64?") as f32,
                    None => panic!("Should've participated?"),
                })
                .enumerate()
                .fold(0.0, |sum, (index, value)| {
                    let multiplier: f32 = match index {
                        0 => 360.0,
                        1 => 60.0,
                        2 => 1.0,
                        3 => 0.01,
                        _ => panic!("More than four captures?"),
                    };

                    sum + (value * multiplier)
                }),
            None => {
                return Err(crate::Error::StaticMsg(
                    "Could not find duration with regex",
                ))
            }
        };

        Ok((path.to_path_buf(), attr.to_string(), total))
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

    // TODO: intro

    // map each frame into an mlt action
    log::info!("Resolving all of the converted frames from their tasks...");
    let frame_tractors: Vec<(String, usize)> = frames
        .map(|frame| match frame {
            Ok(frame) => match frame.into_tractor(&mut mlt, &ctx) {
                Ok((tractor, dur)) => {
                    duration += dur;
                    Ok((tractor, dur))
                }
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        })
        .try_collect()
        .await?;

    // TODO: outro

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
