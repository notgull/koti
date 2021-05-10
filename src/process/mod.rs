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
    util::{ImmediateOrTask, MapFuture},
    Frame,
};
use futures_lite::{
    future,
    stream::{self, Stream, StreamExt},
};
use quick_xml::{
    events::{BytesDecl, BytesEnd, BytesStart, Event},
    Writer,
};
use std::{io::BufWriter, os::unix::ffi::OsStrExt, sync::Arc};
use tokio::{fs::File, process::Command};

mod frame;
pub mod overlay;
pub mod tts;

#[inline]
fn seconds_to_frames(s: f32) -> usize {
    const FPS: f32 = 29.97;
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

    // collect all of the frames into a vector of tasks that are turning those frames into real stuff
    let ctx_clone = ctx.clone();
    let frames_tasks: Vec<_> = frames
        .map(move |f| tokio::spawn(frame::ConvertedFrame::from_frame(f, ctx_clone.clone())))
        .collect()
        .await;

    // collect all of the converted frames and add them to the melt command
    log::info!("Resolving all of the converted frames from their tasks...");
    let frames: Vec<frame::ConvertedFrame> = stream::iter(frames_tasks.into_iter())
        .then(std::convert::identity)
        .map(|e| match e {
            Ok(e) => e,
            Err(e) => Err(crate::Error::Join(e)),
        })
        .try_collect()
        .await?;
    let duration: f32 = frames.iter().map(|f| f.duration()).sum();

    // open an xml file
    let xmlpath = basedir.join("project.mlt");
    let xmlfile = File::create(&xmlpath).await?;
    let mut xreader = Writer::new(BufWriter::new(xmlfile.into_std().await));

    let videopath = basedir.join("video.webm");
    let videopathclone = videopath.clone();

    // write to xml file
    let ctx_clone = ctx.clone();
    tokio::task::spawn_blocking(move || {
        xreader.write_event(Event::Decl(BytesDecl::new(b"1.0", Some(b"utf-8"), None)))?;

        // mlt opener
        let basedir_utf8 = basedir.as_os_str().to_str().expect("Basedir is not utf-8");
        let videopath_utf8 = videopathclone
            .as_os_str()
            .to_str()
            .expect("Videopath is not utf-8?");
        let opener = BytesStart::borrowed_name(b"mlt".as_ref()).with_attributes(vec![
            (s!(b"title"), s!(b"King of the Internet")),
            (s!(b"LC_NUMERIC"), s!(b"en_US.UTF-8")),
            (s!(b"root"), basedir_utf8.as_bytes()),
            (s!(b"producer"), s!(b"outpile")),
        ]);

        xreader.write_event(Event::Start(opener.to_borrowed()))?;

        let (video_width, video_height) = ctx_clone.video_size();
        let (video_width, video_height) = (video_width.to_string(), video_height.to_string());

        // set up a profile
        xreader.write_event(Event::Empty(
            BytesStart::borrowed_name(b"profile").with_attributes(vec![
                (s!(b"frame_rate_num"), s!(b"30000")),
                (s!(b"sample_aspect_num"), s!(b"1")),
                (s!(b"display_aspect_den"), s!(b"9")),
                (s!(b"colorspace"), s!(b"709")),
                (s!(b"progressive"), s!(b"1")),
                (s!(b"display_aspect_num"), s!(b"16")),
                (s!(b"frame_rate_den"), s!(b"1001")),
                (s!(b"width"), video_width.as_bytes()),
                (s!(b"height"), video_height.as_bytes()),
                (s!(b"sample_aspect_den"), s!(b"1")),
            ]),
        ))?;

        let framecount = seconds_to_frames(duration);
        let frames_s = framecount.to_string();

        // set up a consumer that outputs the video
        xreader.write_event(Event::Empty(
            BytesStart::borrowed_name(b"consumer").with_attributes(vec![
                (s!(b"f"), s!(b"webm")),
                (s!(b"cpu-used"), s!(b"4")),
                (s!(b"crf"), s!(b"23")),
                (s!(b"aq"), s!(b"6")),
                (s!(b"max-intra-rate"), s!(b"1000")),
                (s!(b"target"), videopath_utf8.as_bytes()),
                (s!(b"threads"), s!(b"0")),
                (s!(b"real_time"), s!(b"-3")),
                (s!(b"mlt_service"), s!(b"avformat")),
                (s!(b"vcodec"), s!(b"libvpx")),
                (s!(b"quality"), s!(b"good")),
                (s!(b"acodec"), s!(b"libvorbis")),
                (s!(b"in"), s!(b"0")),
                (s!(b"out"), frames_s.as_bytes()),
            ]),
        ))?;

        for f in frames {
            f.add_frame_to_melt(&mut xreader)?;
        }

        xreader.write_event(Event::End(opener.to_end()))?;

        crate::Result::Ok(())
    })
    .await??;

    // run the melt command
    log::info!("Running melt command...");

    Ok(())
}
