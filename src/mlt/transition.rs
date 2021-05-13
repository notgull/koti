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

use crate::util::cow_str_into_bytes;
use quick_xml::events::{attributes::Attribute, BytesEnd, BytesStart, BytesText, Event};
use std::{array::IntoIter as ArrayIter, borrow::Cow, collections::HashMap, iter};

#[derive(Debug)]
pub struct Transition {
    name: Cow<'static, str>,
    a_track: Cow<'static, str>,
    b_track: Cow<'static, str>,
    start: usize,
    end: usize,
    properties: HashMap<Cow<'static, str>, Cow<'static, str>>,
}

impl Transition {
    #[inline]
    pub fn new<
        N: Into<Cow<'static, str>>,
        A: Into<Cow<'static, str>>,
        B: Into<Cow<'static, str>>,
    >(
        name: N,
        a_track: A,
        b_track: B,
        start: usize,
        end: usize,
    ) -> Self {
        Self {
            name: name.into(),
            a_track: a_track.into(),
            b_track: b_track.into(),
            properties: HashMap::new(),
            start,
            end,
        }
    }

    #[inline]
    pub fn property<K: Into<Cow<'static, str>>, V: Into<Cow<'static, str>>>(
        mut self,
        propkey: K,
        propvalue: V,
    ) -> Self {
        self.properties.insert(propkey.into(), propvalue.into());
        self
    }

    #[inline]
    pub fn into_events(self) -> impl Iterator<Item = Event<'static>> {
        let opener = BytesStart::borrowed_name(b"transition").with_attributes(ArrayIter::new([
            Attribute {
                key: b"in".as_ref(),
                value: self.start.to_string().into_bytes().into(),
            },
            Attribute {
                key: b"out".as_ref(),
                value: self.end.to_string().into_bytes().into(),
            },
        ]));
        let closer = BytesEnd::borrowed(b"transition");

        let Self {
            name,
            a_track,
            b_track,
            properties,
            ..
        } = self;

        let service_opener =
            BytesStart::borrowed_name(b"property").with_attributes(iter::once(Attribute {
                key: b"name".as_ref(),
                value: b"mlt_service".as_ref().into(),
            }));
        let service_text = BytesText::from_escaped_str(name);
        let service_closer = BytesEnd::borrowed(b"property");

        let a_opener =
            BytesStart::borrowed_name(b"property").with_attributes(iter::once(Attribute {
                key: b"name".as_ref(),
                value: b"a_track".as_ref().into(),
            }));
        let a_text = BytesText::from_escaped_str(a_track);
        let a_closer = BytesEnd::borrowed(b"property");

        let b_opener =
            BytesStart::borrowed_name(b"property").with_attributes(iter::once(Attribute {
                key: b"name".as_ref(),
                value: b"b_track".as_ref().into(),
            }));
        let b_text = BytesText::from_escaped_str(b_track);
        let b_closer = BytesEnd::borrowed(b"property");

        ArrayIter::new([
            Event::Start(opener),
            Event::Start(service_opener),
            Event::Text(service_text),
            Event::End(service_closer),
            Event::Start(a_opener),
            Event::Text(a_text),
            Event::End(a_closer),
            Event::Start(b_opener),
            Event::Text(b_text),
            Event::End(b_closer),
        ])
        .chain(properties.into_iter().flat_map(|(propkey, propvalue)| {
            let propopen =
                BytesStart::borrowed_name(b"property").with_attributes(iter::once(Attribute {
                    key: b"name".as_ref(),
                    value: cow_str_into_bytes(propkey),
                }));
            let proptext = BytesText::from_escaped_str(propvalue);
            let propend = BytesEnd::borrowed(b"property");
            ArrayIter::new([
                Event::Start(propopen),
                Event::Text(proptext),
                Event::End(propend),
            ])
        }))
        .chain(iter::once(Event::End(closer)))
    }
}
