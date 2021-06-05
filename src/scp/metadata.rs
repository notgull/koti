// MIT/Apache2 License

use quick_xml::{
    events::{BytesEnd, BytesStart, Event},
    Reader,
};

/// Find the list of authors for a particular entry.
#[inline]
pub fn find_authors(entry: &str, metadata: &str) -> Vec<String> {
    metadata_entries(metadata)
        .filter_map(|MetadataEntry { page_name, author }| {
            if page_name == entry {
                Some(author)
            } else {
                None
            }
        })
        .collect()
}

/// Entry in the metadata.
struct MetadataEntry {
    page_name: String,
    author: String,
}

/// Gets an iterator that goes over the HTML of the attribution metadata.
#[inline]
fn metadata_entries(page_text: &str) -> impl Iterator<Item = MetadataEntry> + Send {
    genawaiter::sync::Gen::new(move |co| async move {
        let mut reader = Reader::from_reader(page_text.as_bytes());
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
                event => {
                    if let Some(entry) = state.process(event) {
                        co.yield_(entry).await;
                    }
                }
            }
        }
    })
    .into_iter()
}

enum StateMachine {
    LookingForTable,
    LookingForTr,
    LookingForName,
    LookingForNameText,
    FoundName(String),
    LookingForAuthor(String),
    LookingForAuthorText(String),
}

impl Default for StateMachine {
    #[inline]
    fn default() -> Self {
        Self::LookingForTable
    }
}

impl StateMachine {
    #[inline]
    fn process(&mut self, event: Event<'_>) -> Option<MetadataEntry> {
        match mem::replace(self, Self::LookingForTable) {
            Self::LookingForTable => match event {
                Event::Start(start) if start.name() == b"table" => {
                    *self = Self::LookingForTr;
                }
                _ => {
                    *self = Self::LookingForTable;
                }
            },
            Self::LookingForTr => match event {
                Event::Start(start) if start.name() == b"tr" => {
                    *self = Self::LookingForName;
                }
                Event::End(end) if end.name() == b"table" => {
                    *self = Self::LookingForTable;
                }
                _ => {
                    *self = Self::LookingForTr;
                }
            },
            Self::LookingForName => match event {
                Event::Start(start) if start.name() == b"td" => {
                    *self = Self::LookingForNameText;
                }
                Event::End(end) if end.name() == b"tr" => {
                    *self = Self::LookingForTr;
                }
                _ => *self = Self::LookingForName,
            },
            Self::LookingForNameText => match event {
                Event::Text(txt) => {
                    *self = Self::FoundName(String::from_utf8(txt.escaped().to_vec()).unwrap());
                }
                Event::End(end) if end.name() == b"tr" => {
                    *self = Self::LookingForTr;
                }
                _ => {
                    *self = Self::LookingForNameText;
                }
            },
            Self::FoundName(txt) => match event {
                Event::End(end) if end.name() == b"td" => {
                    *self = Self::LookingForAuthor(txt);
                }
                Event::End(end) if end.name() == b"tr" => {
                    *self = Self::LookingForTr;
                }
                _ => {
                    *self = Self::FoundName(txt);
                }
            },
            Self::LookingForAuthor(txt) => match event {
                Event::Start(start) if start.name() == b"td" => {
                    *self = Self::LookingForAuthorText(txt);
                }
                Event::End(end) if end.name() == b"tr" => {
                    *self = Self::LookingForTr;
                }
                _ => {
                    *self = Self::LookingForAuthor(txt);
                }
            },
            Self::LookingForAuthorText(txt) => match event {
                Event::Text(txt2) => {
                    *self = Self::LookingForTr;
                    return Some(MetadataEntry {
                        page_name: txt,
                        author: String::from_utf8(txt2.escaped().to_vec()).unwrap(),
                    });
                }
                Event::End(end) if end.name() == b"tr" => {
                    *self = Self::LookingForTr;
                }
                _ => {
                    *self = LookingForAuthorText(txt);
                }
            },
        }

        None
    }
}
