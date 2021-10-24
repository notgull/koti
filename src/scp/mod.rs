// MIT/Apache2 License

mod metadata;
mod title;

use nanorand::{RNG, tls_rng};
use quick_xml::{Event, Reader};
use reqwest::Client;
use std::borrow::Cow;

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
    let names = metadata::find_authors(&format!("scp-{:0>3}", number), &attribution_metadata);
    match names.is_empty() {
        false => Ok(names),
        true => {
            Err(crate::Error::StaticMsg("TODO: figure out how to get the author here"))
        }
    }
}

struct Entry {
    number: u16,
    title: String,
    authors: Vec<String>,
    html: String,
}

// figure out the number and title for an SCP
#[inline]
async fn get_number_and_title(client: &Client) -> crate::Result<Entry> {
    loop {
        let number = tls_rng().generate::<u16>().saturating_add(1);
        let title = format!("scp-{:0>3}", number);
        
        // figure out if the page exists
        let html = match client
        .get(format!("http://www.scpwiki.com/{}", &title))
        .send()
        .await {
            Err(_) => continue,
            Ok(response) if response.status().is_success() => { response.text().await? }, 
            Ok(_) => continue,
        };

        let a = authors(client, number).await?; 

        // the page exists, get a title
        let series_page_name = match number / 1000 {
            0 => Cow::Borrowed("https:/www.scpwiki.com/scp-series"),
            i => Cow::Owned(format!("https:/www.scpwiki.com/scp-series-{}", i)),
        };
        let series_page = client.get(series_page_name).send().await?.text().await?;  

        let t = title::get_title(client, number)?;

        unimplemented!()
    };
}
