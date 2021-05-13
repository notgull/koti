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

#[derive(Debug, Clone)]
pub struct Filter {
    name: Cow<'static, str>,
    properties: HashMap<Cow<'static, str>, Cow<'static, str>>,
}

impl Filter {
    #[inline]
    pub fn new<S: Into<Cow<'static, str>>>(name: S) -> Self {
        Self {
            name: name.into(),
            properties: HashMap::new(),
        }
    }

    #[inline]
    pub fn property<K: Into<Cow<'static, str>>, V: Into<Cow<'static, str>>>(
        mut self,
        key: K,
        value: V,
    ) -> Self {
        self.properties.insert(key.into(), value.into());
        self
    }

    #[inline]
    pub fn into_events(self) -> impl Iterator<Item = Event<'static>> {
        let Self { name, properties } = self;

        let opener = BytesStart::borrowed_name(b"filter");
        let closer = BytesEnd::borrowed(b"filter");
        let mlt_service_opener = BytesStart::borrowed_name(b"property")
            .with_attributes(iter::once(("name", "mlt_service")));
        let mlt_service_closer = BytesEnd::borrowed(b"property");
        let mlt_service_text = BytesText::from_escaped_str(name);

        // chain the iterator
        ArrayIter::new([
            Event::Start(opener),
            Event::Start(mlt_service_opener),
            Event::Text(mlt_service_text),
            Event::End(mlt_service_closer),
        ])
        .chain(properties.into_iter().flat_map(|(propkey, propvalue)| {
            let propopener =
                BytesStart::borrowed_name(b"property").with_attributes(iter::once(Attribute {
                    key: b"name",
                    value: cow_str_into_bytes(propkey),
                }));
            let proptext = BytesText::from_escaped_str(propvalue);
            let propcloser = BytesEnd::borrowed(b"property");

            ArrayIter::new([
                Event::Start(propopener),
                Event::Text(proptext),
                Event::End(propcloser),
            ])
        }))
        .chain(iter::once(Event::End(closer)))
    }
}
