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

use super::{Filter, Transition};
use quick_xml::events::{attributes::Attribute, BytesEnd, BytesStart, Event};
use std::{
    array::IntoIter as ArrayIter,
    iter,
    sync::atomic::{AtomicUsize, Ordering},
};

static ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct Tractor {
    multitrack: Vec<String>,
    filters: Vec<Filter>,
    transitions: Vec<Transition>,
    id: String,
}

impl Tractor {
    #[inline]
    pub fn new() -> Self {
        Self {
            multitrack: vec![],
            filters: vec![],
            transitions: vec![],
            id: format!("tractor{}", ID.fetch_add(1, Ordering::SeqCst)),
        }
    }

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn add_track(&mut self, track_id: String) {
        self.multitrack.push(track_id);
    }

    #[inline]
    pub fn add_filter(&mut self, filter: Filter) {
        self.filters.push(filter);
    }

    #[inline]
    pub fn add_transition(&mut self, trans /* notgull says trans rights */: Transition) {
        self.transitions.push(trans);
    }

    #[inline]
    pub fn into_events(self) -> impl Iterator<Item = Event<'static>> {
        let Self {
            multitrack,
            filters,
            transitions,
            id,
        } = self;
        let opener = BytesStart::borrowed_name(b"tractor").with_attributes(iter::once(Attribute {
            key: b"id".as_ref(),
            value: id.into_bytes().into(),
        }));
        let closer = BytesEnd::borrowed(b"tractor");
        let mt_opener = BytesStart::borrowed_name(b"multitrack");
        let mt_closer = BytesEnd::borrowed(b"multitrack");

        ArrayIter::new([Event::Start(opener), Event::Start(mt_opener)])
            .chain(multitrack.into_iter().map(|track| {
                Event::Empty(
                    BytesStart::borrowed_name(b"track").with_attributes(iter::once(Attribute {
                        key: b"producer".as_ref(),
                        value: track.into_bytes().into(),
                    })),
                )
            }))
            .chain(iter::once(Event::End(mt_closer)))
            .chain(filters.into_iter().flat_map(|filter| filter.into_events()))
            .chain(
                transitions
                    .into_iter()
                    .flat_map(|transition| transition.into_events()),
            )
            .chain(iter::once(Event::End(closer)))
    }
}
