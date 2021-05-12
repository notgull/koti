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

use super::Filter;
use quick_xml::events::{attributes::Attribute, BytesEnd, BytesStart, Event};
use std::{
    array::IntoIter as ArrayIter,
    iter,
    sync::atomic::{AtomicUsize, Ordering},
};

static ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct Playlist {
    entries: Vec<PlaylistEntry>,
    filters: Vec<Filter>,
    id: String,
}

impl Playlist {
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: vec![],
            filters: vec![],
            id: format!("playlist{}", ID.fetch_add(1, Ordering::SeqCst)),
        }
    }

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn push_blank(&mut self, length: usize) {
        self.entries.push(PlaylistEntry::Blank(length));
    }

    #[inline]
    pub fn push_entry(&mut self, id: String, start: usize, end: usize) {
        self.entries.push(PlaylistEntry::Video { id, start, end });
    }

    #[inline]
    pub fn push_filter(&mut self, filter: Filter) {
        self.filters.push(filter);
    }

    #[inline]
    pub fn into_events(self) -> impl Iterator<Item = Event<'static>> {
        let Self {
            filters,
            entries,
            id,
        } = self;
        let playlist =
            BytesStart::borrowed_name(b"playlist").with_attributes(iter::once(Attribute {
                key: b"id",
                value: id.into_bytes().into(),
            }));
        let playlist_end = BytesEnd::borrowed(b"playlist");

        iter::once(Event::Start(playlist))
            .chain(entries.into_iter().map(|entry| match entry {
                PlaylistEntry::Blank(length) => Event::Empty(
                    BytesStart::borrowed_name(b"blank").with_attributes(iter::once(Attribute {
                        key: b"length",
                        value: length.to_string().into_bytes().into(),
                    })),
                ),
                PlaylistEntry::Video { id, start, end } => Event::Empty(
                    BytesStart::borrowed_name(b"entry").with_attributes(ArrayIter::new([
                        Attribute {
                            key: b"producer",
                            value: id.into_bytes().into(),
                        },
                        Attribute {
                            key: b"in",
                            value: format!("{}", start).into_bytes().into(),
                        },
                        Attribute {
                            key: b"out",
                            value: format!("{}", end).into_bytes().into(),
                        },
                    ])),
                ),
            }))
            .chain(filters.into_iter().flat_map(|f| f.into_events()))
            .chain(iter::once(Event::End(playlist_end)))
    }
}

#[derive(Debug)]
enum PlaylistEntry {
    Blank(usize),
    Video {
        id: String,
        start: usize,
        end: usize,
    },
}
