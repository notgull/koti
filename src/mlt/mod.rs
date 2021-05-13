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

mod filter;
mod playlist;
mod producer;
mod tractor;
mod transition;

pub use filter::Filter;
pub use transition::Transition;

use crate::process::FPS;
use playlist::Playlist;
use producer::Producer;
use quick_xml::{
    events::{attributes::Attribute, BytesDecl, BytesEnd, BytesStart, Event},
    Writer,
};
use std::{
    array::IntoIter as ArrayIter,
    io::BufWriter,
    iter, mem,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{fs::File, process::Command};
use tractor::Tractor;

#[doc(hidden)]
#[macro_export]
macro_rules! s {
    ($e: expr) => {{
        ($e).as_ref()
    }};
}

const FRAME_RATE_DEN: usize = 1001;

/// Manager for the MLT xml file.
pub struct Mlt<'a> {
    basedir: &'a Path,
    events: Vec<Event<'static>>,
    video_size: (usize, usize),
}

pub enum PlaylistEntry {
    Blank(usize),
    Producer {
        id: String,
        start: usize,
        end: usize,
    },
}

impl<'a> Mlt<'a> {
    #[inline]
    pub fn new(basedir: &'a Path, video_width: usize, video_height: usize) -> Self {
        Self {
            basedir,
            events: vec![],
            video_size: (video_width, video_height),
        }
    }

    /// Returns the ID
    #[inline]
    pub fn add_producer(&mut self, resource: PathBuf) -> String {
        let prod = Producer::new(resource);
        let id = prod.id().to_string();
        self.events.extend(prod.into_events());
        id
    }

    /// Returns the ID
    #[inline]
    pub fn add_playlist<I, J>(&mut self, producers: I, filters: J) -> String
    where
        I: IntoIterator<Item = PlaylistEntry>,
        J: IntoIterator<Item = Filter>,
    {
        let mut playlist = Playlist::new();
        producers.into_iter().for_each(|p| match p {
            PlaylistEntry::Blank(b) => playlist.push_blank(b),
            PlaylistEntry::Producer { id, start, end } => playlist.push_entry(id, start, end),
        });
        filters.into_iter().for_each(|f| playlist.push_filter(f));
        let id = playlist.id().to_string();

        self.events.extend(playlist.into_events());
        id
    }

    #[inline]
    pub fn add_tractor<I, J>(&mut self, tracks: I, filters: J) -> String
    where
        I: IntoIterator<Item = String>,
        J: IntoIterator<Item = Filter>,
    {
        self.add_tractor_with_transitions(tracks, filters, iter::empty())
    }

    /// Returns the ID
    #[inline]
    pub fn add_tractor_with_transitions<I, J, T>(
        &mut self,
        tracks: I,
        filters: J,
        transitions: T,
    ) -> String
    where
        I: IntoIterator<Item = String>,
        J: IntoIterator<Item = Filter>,
        T: IntoIterator<Item = Transition>,
    {
        let mut tractor = Tractor::new();
        tracks.into_iter().for_each(|t| tractor.add_track(t));
        filters.into_iter().for_each(|f| tractor.add_filter(f));
        transitions
            .into_iter()
            .for_each(|t| tractor.add_transition(t));

        let id = tractor.id().to_string();

        self.events.extend(tractor.into_events());
        id
    }

    #[inline]
    pub fn into_events(
        self,
        main_tractor: String,
        duration: usize,
    ) -> (PathBuf, impl Iterator<Item = Event<'static>>) {
        let Self {
            events,
            basedir,
            video_size: (video_width, video_height),
        } = self;
        let videopath = basedir.join("koti.webm");

        // mlt opener
        let opener = BytesStart::borrowed_name(b"mlt").with_attributes(ArrayIter::new([
            Attribute {
                key: b"title".as_ref(),
                value: s!(b"King of the Internet").into(),
            },
            Attribute {
                key: b"producer".as_ref(),
                value: main_tractor.into_bytes().into(),
            },
            Attribute {
                key: b"root".as_ref(),
                value: path_to_utf8(basedir).to_string().into_bytes().into(),
            },
        ]));
        let closer = BytesEnd::borrowed(b"mlt");

        // width/height/fps profile
        let (frame_rate_num) = (FPS * (FRAME_RATE_DEN as f32)).ceil() as usize;
        let (video_width, video_height) = (video_width.to_string(), video_height.to_string());
        let (frame_rate_num, frame_rate_den) =
            (frame_rate_num.to_string(), FRAME_RATE_DEN.to_string());
        let profile = BytesStart::borrowed_name(b"profile").with_attributes(ArrayIter::new([
            Attribute {
                key: b"width".as_ref(),
                value: video_width.into_bytes().into(),
            },
            Attribute {
                key: b"height".as_ref(),
                value: video_height.into_bytes().into(),
            },
            Attribute {
                key: b"frame_rate_num".as_ref(),
                value: frame_rate_num.into_bytes().into(),
            },
            Attribute {
                key: b"frame_rate_den".as_ref(),
                value: frame_rate_den.into_bytes().into(),
            },
        ]));

        // consumer to output video from
        let duration = duration.to_string();
        let consumer = BytesStart::borrowed_name(b"consumer").with_attributes(ArrayIter::new([
            Attribute {
                key: b"f".as_ref(),
                value: s!(b"webm").into(),
            },
            Attribute {
                key: b"target".as_ref(),
                value: path_to_utf8(&videopath).to_string().into_bytes().into(),
            },
            Attribute {
                key: b"in".as_ref(),
                value: s!(b"0").into(),
            },
            Attribute {
                key: b"out".as_ref(),
                value: duration.into_bytes().into(),
            },
            Attribute {
                key: b"mlt_service".as_ref(),
                value: b"avformat".as_ref().into(),
            },
        ]));

        (
            videopath,
            ArrayIter::new([
                Event::Decl(BytesDecl::new(b"1.0", Some(b"utf-8"), None)),
                Event::Start(opener),
                Event::Empty(profile),
                Event::Empty(consumer),
            ])
            .chain(events.into_iter())
            .chain(iter::once(Event::End(closer))),
        )
    }

    #[inline]
    pub async fn save(
        self,
        main_tractor: String,
        duration: usize,
    ) -> crate::Result<(PathBuf, PathBuf)> {
        let outpath = self.basedir.join("project.mlt");
        // collect the events into a vec
        let (videopath, events) = self.into_events(main_tractor, duration);

        // create the xml writer and write the events
        let file = File::create(&outpath).await?;
        let file = BufWriter::new(file.into_std().await);
        let mut writer = Writer::new(file);
        log::info!("Writing XML describing video...");
        tokio::task::spawn_blocking(move || {
            events
                .map(move |event| writer.write_event(event).map_err(|e| crate::Error::Xml(e)))
                .collect::<crate::Result>()
        })
        .await??;
        Ok((outpath, videopath))
    }

    #[inline]
    pub async fn run(self, main_tractor: String, duration: usize) -> crate::Result<PathBuf> {
        let basedir = self.basedir;
        let (outpath, videopath) = self.save(main_tractor, duration).await?;

        // start the melt command with the outpath (xml) as the parameter
        log::info!("Running melt...");
        let mut output = Command::new("melt")
            .current_dir(basedir)
            .arg(outpath)
            //            .stdout(Stdio::inherit())
            //            .stderr(Stdio::inherit())
            .output()
            .await?;
        log::info!("melt has finished!");

        if !output.status.success() {
            return Err(crate::Error::StaticMsg("Melt failed"));
        }

        Ok(videopath)
    }
}

/// Convert path to str convenience function.
#[inline]
pub fn path_to_utf8(basedir: &Path) -> &str {
    basedir
        .as_os_str()
        .to_str()
        .expect("Path is not utf-8, for whatever reason?")
}

#[inline]
pub fn pathbuf_to_utf8(basedir: PathBuf) -> String {
    basedir
        .into_os_string()
        .into_string()
        .expect("Path is not utf-8?")
}

#[inline]
pub fn subpath_to_utf8(basedir: &Path, filename: &str) -> String {
    basedir
        .join(filename)
        .into_os_string()
        .into_string()
        .expect("Path is not utf-8?")
}
