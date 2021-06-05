// MIT/Apache2 License

mod metadata;

use quick_xml::{Event, Reader};
use reqwest::Client;

// figure out the author(s) for a particular SCP
#[inline]
async fn authors(client: &Client, number: u16) -> crate::Result<Vec<String>> {
    // first, try for the "attribution metadata" page
    let attribution_metadata = client
        .get("http://www.scpwiki.com/attribution-metadata")
        .send()
        .await?
        .text()
        .await?;
    // then, read through it to see if we can find our number
    let names = metadata::find_authors(&format!("scp-{:0>3}", number), &attribution_metadata));
    match names.is_empty() {
        false => Ok(names),
        true => {
            Ok(vec![]),
        }
    }
}
