// GNU AGPL v3

use quick_xml::{
    events::{BytesEnd, BytesStart, Event},
    Reader,
};
use std::borrow::Cow;

/// Given the outer HTML of a comment thing element, get a list of ID's corresponding to its children.
pub fn direct_children(html: String) -> impl Iterator<Item = String> + Send {
    genawaiter::sync::Gen::new(move |co| async move {
        let mut reader = Reader::from_reader(html.as_bytes());
        let mut state = StateMachine::default();

        let mut buf = vec![];

        loop {
            let event = match reader.read_event(&mut buf) {
                Ok(event) => event,
                Err(e) => {
                    log::error!("XML Error: {}", e);
                    buf.clear();
                    continue;
                }
            };

            match event {
                Event::Eof => break,
                Event::Start(st) => {
                    if let Some(id) = state.process_start(st) {
                        if let Ok(id) = String::from_utf8(id) {
                            co.yield_(id).await;
                        }
                    }
                }
                Event::End(e) => state.process_end(e),
                _ => (),
            }

            buf.clear();
        }
    })
    .into_iter()
}

enum StateMachine {
    IgnoringTopLevel(usize),
    ChildTag(usize, usize),
    SitetableTag(usize, usize, usize),
    ThingTag(usize, usize, usize, usize),
}

impl Default for StateMachine {
    #[inline]
    fn default() -> Self {
        Self::IgnoringTopLevel(0)
    }
}

impl StateMachine {
    #[inline]
    fn depth(&mut self) -> &mut usize {
        match self {
            Self::IgnoringTopLevel(d) => d,
            Self::ChildTag(c, ..) => c,
            Self::SitetableTag(s, ..) => s,
            Self::ThingTag(t, ..) => t,
        }
    }

    #[inline]
    fn process_start<'a>(&mut self, bytes: BytesStart<'a>) -> Option<Vec<u8>> {
        if bytes.name() == b"div" {
            if let Some(class) = bytes.attributes().find_map(|a| {
                if let Ok(a) = a {
                    if a.key == b"class" {
                        Some(a.value)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }) {
                if subsequence(&class, b"child") {
                    if let Self::IgnoringTopLevel(i) = self {
                        *self = Self::ChildTag(0, *i);
                        return None;
                    }
                }

                if subsequence(&class, b"sitetable") {
                    if let Self::ChildTag(c, i) = self {
                        *self = Self::SitetableTag(0, *c, *i);
                        return None;
                    }
                }

                if subsequence(&class, b"comment") {
                    if let Self::SitetableTag(s, c, i) = self {
                        *self = Self::ThingTag(0, *s, *c, *i);
                        return bytes.attributes().find_map(|a| {
                            if let Ok(a) = a {
                                if a.key == b"id" {
                                    Some(a.value.into_owned())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        });
                    }
                }
            }
        }

        *self.depth() += 1;
        None
    }

    #[inline]
    fn process_end<'a>(&mut self, bytes: BytesEnd<'a>) {
        let d = self.depth();
        match d.checked_sub(1) {
            Some(a) => {
                *d = a;
            }
            None => match self {
                Self::IgnoringTopLevel(_) => panic!(),
                Self::ChildTag(_, i) => {
                    *self = Self::IgnoringTopLevel(*i);
                }
                Self::SitetableTag(_, c, i) => {
                    *self = Self::ChildTag(*c, *i);
                }
                Self::ThingTag(_, s, c, i) => {
                    *self = Self::SitetableTag(*s, *c, *i);
                }
            },
        }
    }
}

#[inline]
fn subsequence(container: &[u8], test: &[u8]) -> bool {
    container
        .windows(test.len())
        .find(|window| *window == test)
        .is_some()
}
