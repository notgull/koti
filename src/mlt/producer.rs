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

use super::pathbuf_to_utf8;
use quick_xml::events::{attributes::Attribute, BytesEnd, BytesStart, BytesText, Event};
use std::{
    array::IntoIter as ArrayIter,
    iter,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
pub struct Producer {
    resource: PathBuf,
    id: String,
}

impl Producer {
    #[inline]
    pub fn new(resource: PathBuf) -> Self {
        Self {
            resource,
            id: format!("producer{}", ID.fetch_add(1, Ordering::SeqCst)),
        }
    }

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn into_events(self) -> impl Iterator<Item = Event<'static>> {
        let wrapper =
            BytesStart::borrowed_name(b"producer").with_attributes(iter::once(Attribute {
                key: b"id",
                value: self.id.clone().into_bytes().into(),
            }));
        let property1 = BytesStart::borrowed_name(b"property")
            .with_attributes(iter::once(("name", "resource")));
        let wrapper_end = BytesEnd::borrowed(b"producer");
        let property1_end = BytesEnd::borrowed(b"property");

        ArrayIter::new([
            Event::Start(wrapper),
            Event::Start(property1),
            Event::Text(BytesText::from_escaped(
                pathbuf_to_utf8(self.resource).into_bytes(),
            )),
            Event::End(property1_end),
            Event::End(wrapper_end),
        ])
    }
}
