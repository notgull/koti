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

use crate::{
    context::Context,
    frame::Frame,
    util::{self, ArcWebElement},
};
use futures_lite::stream::{self, Stream, StreamExt};
use nanorand::{tls_rng, RNG};
use std::{
    array::IntoIter as ArrayIter,
    cmp,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use thirtyfour::{common::types::ElementRect, prelude::*};

static GLOBAL_NUMBER: AtomicUsize = AtomicUsize::new(0);

#[derive(Copy, Clone)]
enum CommentStatus {
    Toplevel,
    LowerLevel {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

impl Default for CommentStatus {
    #[inline]
    fn default() -> CommentStatus {
        CommentStatus::Toplevel
    }
}

type RoC = crate::Result<Option<Vec<Frame>>>;

// note: this used to return a Pin<Box<dyn Stream>>, but i couldn't make the lifetimes work
#[inline]
fn report_on_comment<'c>(
    elem: ArcWebElement,
    basedir: Arc<Path>,
    comment_threshold: i64,
    reply_threshold: i64,
    status: CommentStatus,
) -> Pin<Box<dyn Future<Output = RoC> + Send + 'c>> {
    #[inline]
    async fn report_on_comment(
        elem: ArcWebElement,
        basedir: Arc<Path>,
        comment_threshold: i64,
        reply_threshold: i64,
        status: CommentStatus,
    ) -> RoC {
        let driver = elem.as_owner();
        let element = elem.elem();

        if matches!(status, CommentStatus::Toplevel) {
            element.scroll_into_view().await?;
        }

        // if the comment has less than comment_threshold upvotes, don't bother
        let commentbody = element.find_element(By::ClassName("entry")).await?;
        let score = commentbody.find_element(By::ClassName("score")).await?;
        match score.get_attribute("title").await? {
            None => return Ok(None),
            Some(score) => {
                let score = i64::from_str(&score).map_err(|_| crate::Error::NumParseError)?;
                if score
                    < if matches!(status, CommentStatus::Toplevel) {
                        comment_threshold
                    } else {
                        reply_threshold
                    }
                {
                    return Ok(None);
                }
            }
        }

        // turn this comment into a frame
        let ElementRect {
            x,
            y,
            width,
            height,
        } = commentbody.rect().await?;
        let (x, y, width, height) = (
            x.floor() as u32,
            y.floor() as u32,
            width.ceil() as u32,
            height.ceil() as u32,
        );
        let (x, y, width, height): (u32, u32, u32, u32) = match status {
            CommentStatus::Toplevel => (x, y, width, height),
            CommentStatus::LowerLevel {
                x: xtop,
                y: ytop,
                width: widthtop,
                height: heighttop,
            } => (x, y, cmp::max(width, widthtop), height + heighttop + 5),
        };

        // take a screenshot of the element as well as its parent
        let path = basedir.join(&format!(
            "comment{}.png",
            GLOBAL_NUMBER.fetch_add(1, Ordering::Relaxed)
        ));
        util::cropped_screenshot(&*driver, x, y, width, height, &path).await?;
        let myframe = Frame {
            // concat all the paragraphs
            tts: stream::iter(
                commentbody
                    .find_elements(By::Tag("p"))
                    .await?
                    .into_iter()
                    .skip(1usize),
            )
            .then(|e| async move { e.text().await })
            .filter_map(Result::ok)
            .collect::<String>()
            .await,
            overlaid: String::new(),
            imagepath: Some(path.to_path_buf()),
            imagefadesin: false,
            persists_after_tts: 1.5,
        };

        // run this code on all the child comments
        let driver_clone = driver.clone();
        let children = element
            .find_elements(By::XPath(
                "/div[contains(@class, 'child')]/div[contains(@class, 'comment')]",
            ))
            .await?
            .into_iter()
            .map(move |elem| {
                (
                    ArcWebElement::new(driver_clone.clone(), elem.element_id),
                    basedir.clone(),
                )
            })
            .collect::<Vec<_>>();
        // todo: figure out a way to not have to collect() this stream
        let frames = stream::once(myframe)
            .chain(
                stream::iter(children.into_iter())
                    .then(move |(elem, b)| {
                        report_on_comment(
                            elem,
                            b,
                            comment_threshold,
                            reply_threshold,
                            CommentStatus::LowerLevel {
                                x,
                                y,
                                width,
                                height,
                            },
                        )
                    })
                    .filter_map(move |t| {
                        if let Ok(Some(t)) = t {
                            Some(stream::iter(t.into_iter()))
                        } else {
                            None
                        }
                    })
                    .flatten(),
            )
            .collect::<Vec<_>>()
            .await;
        Ok(Some(frames))
    }

    Box::pin(async move { inner(elem, basedir, comment_threshold, reply_threshold, status).await })
}

pub async fn reddit_text_source(
    subreddit: &str,
    upvote_threshold: i64,
    comment_threshold: i64,
    reply_threshold: i64,
    net: &str,
    context: &Context,
) -> crate::Result<impl Stream<Item = Frame> + Send + 'static> {
    // create selenium web driver
    let caps = DesiredCapabilities::firefox();
    let driver = WebDriver::new("http://localhost:4444/wd/hub", caps).await?;
    let basedir = context.basedir().await;

    // open the URL
    driver
        .get(format!(
            "https://old.reddit.com/r/{}/top/?sort=top&t={}",
            subreddit, net
        ))
        .await?;

    // find the elements
    let site_table = driver.find_element(By::Id("siteTable")).await?;
    let items = site_table
        .find_elements(By::Css(".link:not(promoted)"))
        .await?;
    let items: Vec<WebElement<'_>> = stream::iter(items.into_iter())
        .then(|item| {
            async fn get_item_score<'a>(item: &WebElement<'a>) -> crate::Result<Option<i64>> {
                let valelem = item.find_element(By::ClassName("score")).await?;
                let scoreattr = valelem.get_attribute("title").await?;
                Ok(match scoreattr {
                    None => None,
                    Some(score) => {
                        let score: i64 =
                            i64::from_str(&score).map_err(|_| crate::Error::NumParseError)?;
                        Some(score)
                    }
                })
            }

            async move { (get_item_score(&item).await, item) }
        })
        .filter(|(score, _)| {
            if let Ok(Some(score)) = score {
                *score >= upvote_threshold
            } else {
                false
            }
        })
        .map(|(_, item)| item)
        .collect()
        .await;

    // randomly select a thread
    let index: usize = tls_rng().generate_range(0, items.len() - 1);

    // cant move out of vec...
    let item = items.into_iter().skip(index).next().unwrap();
    let title = item.find_element(By::Tag("a")).await?.text().await?;

    context.set_thumbnail(
        format!("/r/{}: {}", subreddit, &title),
        Path::new("reddit_text").to_path_buf(),
    );
    context.set_video_title(format!("{} - /r/{}", title.to_uppercase(), subreddit));

    // take a screenshot of that item and use it as a frame
    let titlescreenname = basedir.join("title.png");
    util::screenshot_item(&driver, &item, &titlescreenname).await?;
    let titleframe = Frame {
        tts: title,
        overlaid: String::new(),
        imagepath: Some(titlescreenname),
        imagefadesin: false,
        persists_after_tts: 1.5,
    };

    // tell driver to go to that index
    let attribute = item
        .get_attribute("data-url")
        .await?
        .expect("No data-url attribute?");
    let url = format!("https://old.reddit.com{}", attribute);
    driver.get(url).await?;

    // find the user's post
    let driver = Arc::new(driver);
    let userpost = driver.find_element(By::ClassName("self")).await?;

    // within the post, there will be paragraphs, turn each of these into a frame
    let driver_clone = driver.clone();
    let paragraphs = driver
        .find_elements(By::Tag("p"))
        .await?
        .into_iter()
        .map(move |element| ArcWebElement::new(driver_clone.clone(), element.element_id))
        .collect::<Vec<_>>();
    let basedir: Arc<Path> = basedir.into_boxed_path().into();
    let basedirclone = basedir.clone();
    let parframes = stream::iter(paragraphs.into_iter())
        .map(move |item| (basedirclone.clone(), item))
        .then(|(basedir, item)| async move {
            let driver = item.as_owner();
            let iteme = item.elem();
            let parscreename = format!(
                "paragraph_{}.png",
                GLOBAL_NUMBER.fetch_add(1, Ordering::SeqCst)
            );
            let parscreename = basedir.join(&parscreename);
            util::screenshot_item(&driver, &iteme, &parscreename)
                .await
                .unwrap();
            Frame {
                tts: iteme.text().await.unwrap(),
                overlaid: String::new(),
                imagepath: Some(parscreename),
                imagefadesin: false,
                persists_after_tts: 1.5,
            }
        });

    // add a frame for the comments
    let comments_frame = Frame {
        tts: "Comments".to_string(),
        overlaid: "Comments".to_string(),
        imagepath: None,
        imagefadesin: false,
        persists_after_tts: 1.5,
    };

    // iterate through the comments and see which ones we want to use
    let driver_clone = driver.clone();
    let comment_frames = stream::iter(
        driver_clone
            .clone()
            .find_element(By::ClassName("sitetable"))
            .await?
            .find_elements(By::XPath("/div[contains(@class, 'comment')]"))
            .await?
            .into_iter()
            .map(move |elem| ArcWebElement::new(driver_clone.clone(), elem.element_id))
            .collect::<Vec<_>>()
            .into_iter(),
    )
    .then(move |elem| {
        report_on_comment(
            elem,
            basedir.clone(),
            comment_threshold,
            reply_threshold,
            Default::default(),
        )
    })
    .filter_map(|r| {
        if let Ok(Some(t)) = r {
            Some(stream::iter(t.into_iter()))
        } else {
            None
        }
    })
    .flatten();

    // we should have all the frames we need, put them together
    Ok(Box::pin(
        stream::once(titleframe)
            .chain(parframes)
            .chain(stream::once(comments_frame))
            .chain(comment_frames),
    ))
}
