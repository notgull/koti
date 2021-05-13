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
    util::{self, timeout, ArcWebElement},
};
use futures_lite::{
    future,
    stream::{self, Stream, StreamExt},
};
use nanorand::{tls_rng, RNG};
use once_cell::sync::OnceCell;
use std::{
    array::IntoIter as ArrayIter,
    cmp,
    future::Future,
    mem,
    path::{Path, PathBuf},
    pin::Pin,
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use thirtyfour::{common::types::ElementRect, error::WebDriverResult, prelude::*};

mod direct_children;
use direct_children::direct_children;

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

/// Represents a comment in a comment thread.
struct Comment {
    elem: ArcWebElement,
    commentbody: ArcWebElement,
    save_basedir: Arc<Path>,
    status: CommentStatus,
    screenshot_rect: OnceCell<(u32, u32, u32, u32)>,
}

impl Comment {
    #[inline]
    async fn new(
        elem: ArcWebElement,
        save_basedir: Arc<Path>,
        status: CommentStatus,
    ) -> crate::Result<Self> {
        let commentbody = elem.elem().find_element(By::ClassName("entry")).await?;
        let commentbody = ArcWebElement::new(elem.as_owner().clone(), commentbody.element_id);
        Ok(Self {
            elem,
            commentbody,
            save_basedir,
            status,
            screenshot_rect: OnceCell::new(),
        })
    }

    #[inline]
    fn is_toplevel(&self) -> bool {
        matches!(&self.status, CommentStatus::Toplevel)
    }

    #[inline]
    async fn score(&self) -> crate::Result<i64> {
        log::info!("Trying to get comment score");
        let score = timeout(
            self.commentbody.elem().find_element(By::ClassName("score")),
            10,
        )
        .await??;
        let score = match score.get_attribute("title").await? {
            None => Err(crate::Error::StaticMsg(
                "Score doesn't exist, likely a blunder",
            )),
            Some(score) => i64::from_str(&score).map_err(|_| crate::Error::NumParseError),
        }?;
        log::info!("Got the score!");
        Ok(score)
    }

    #[inline]
    async fn screenshot_rect(&self) -> crate::Result<(u32, u32, u32, u32)> {
        match self.screenshot_rect.get() {
            Some(res) => Ok(*res),
            None => {
                let ElementRect {
                    x,
                    y,
                    width,
                    height,
                } = self.commentbody.elem().rect().await?;
                let (x, y, width, height) = (
                    x.floor() as u32,
                    y.floor() as u32,
                    width.ceil() as u32,
                    height.ceil() as u32,
                );
                let mut res = match self.status {
                    CommentStatus::Toplevel => (x, y, width, height),
                    CommentStatus::LowerLevel {
                        x: xtop,
                        y: ytop,
                        width: widthtop,
                        height: heighttop,
                    } => (
                        xtop,
                        ytop,
                        cmp::max(width, widthtop),
                        height + heighttop + 15,
                    ),
                };
                res.1 = 0;
                self.screenshot_rect.set(res).ok();
                Ok(res)
            }
        }
    }

    #[inline]
    async fn screenshot(&self) -> crate::Result<PathBuf> {
        log::info!("Screenshotting comment...");
        let path = self.save_basedir.join(&format!(
            "comment{}.png",
            GLOBAL_NUMBER.fetch_add(1, Ordering::Relaxed)
        ));
        let (x, y, width, height) = self.screenshot_rect().await?;
        if self.is_toplevel() {
            self.scroll().await?;
        }
        util::cropped_screenshot(self.elem.as_owner(), x, y, width, height, &path).await?;
        Ok(path)
    }

    #[inline]
    async fn text(&self) -> crate::Result<String> {
        log::info!("Getting comment text");
        Ok(stream::iter(
            self.commentbody
                .elem()
                .find_elements(By::Tag("p"))
                .await?
                .into_iter()
                .skip(1usize),
        )
        .then(|e| async move { e.inner_html().await })
        .filter_map(util::ok_log)
        .collect::<String>()
        .await)
    }

    #[inline]
    async fn scroll(&self) -> crate::Result<()> {
        self.elem.elem().scroll_into_view().await?;
        Ok(())
    }

    #[inline]
    async fn frame(&self) -> crate::Result<Frame> {
        let sspath = self.screenshot().await?;
        let text = self.text().await?;
        log::info!("Got comment text");
        Ok(Frame {
            tts: text,
            overlaid: String::new(),
            imagepath: Some(sspath),
            imagefadesin: false,
            persists_after_tts: 0.5,
        })
    }

    #[inline]
    async fn into_frame(self) -> crate::Result<Frame> {
        self.frame().await
    }

    #[inline]
    async fn direct_children(self) -> crate::Result<impl Stream<Item = Comment> + Send + 'static> {
        log::info!("Calculting direct children for comment...");
        let id = if let Ok(Some(id)) = self.elem.elem().get_attribute("id").await {
            log::info!("Current ID is {}", &id);
            Some(id)
        } else {
            None
        };
        let driver_clone = self.elem.as_owner().clone();
        let basedir_clone = self.save_basedir.clone();
        let (x, y, width, height) = self.screenshot_rect().await?;

        // get the HTML of the element
        let html = self.elem.elem().inner_html().await?;

        // get the children out of it
        let childs = direct_children(html);
        /*let eclone = self.elem.clone();
        let childs = stream::iter(childs)
            .then(move |child_id| eclone.clone().elem().find_element(By::Id(&child_id)))
            .filter_map(util::ok_log)
            .map(move |elem| {
                (
                    ArcWebElement::new(driver_clone.clone(), elem.element_id),
                    basedir_clone.clone(),
                )
            })
            .collect::<Vec<_>>()
            .await;*/

        // equivalent code but it actually compiles
        let mut subelems = vec![];
        let rootelem = self.elem.elem();
        for child_id in childs {
            log::info!("Child is {}", &child_id);

            if id.as_deref() == Some(&child_id) {
                log::warn!("Child ID is equal to our ID");
            }

            let subelem = rootelem.find_element(By::Id(&child_id)).await?;
            let subelem = ArcWebElement::new(driver_clone.clone(), subelem.element_id);
            subelems.push((subelem, basedir_clone.clone()))
        }
        mem::drop(rootelem);

        if subelems.is_empty() {
            log::info!("No direct children found");
        }

        Ok(stream::once(self).chain(
            stream::iter(subelems.into_iter())
                .enumerate()
                .then(move |(index, (elem, basedir))| {
                    Comment::new(
                        elem,
                        basedir,
                        if index == 0 {
                            CommentStatus::LowerLevel {
                                x,
                                y,
                                width,
                                height,
                            }
                        } else {
                            CommentStatus::Toplevel
                        },
                    )
                })
                .filter_map(util::ok_log),
        ))
    }

    #[inline]
    fn children(
        self,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = crate::Result<Pin<Box<dyn Stream<Item = Comment> + Send + 'static>>>,
                > + Send
                + 'static,
        >,
    > {
        Box::pin(async move {
            let s = Box::pin(
                self.direct_children()
                    .await?
                    .enumerate()
                    .then(|(index, comment)| async move {
                        // first child doesn't need it
                        if index == 0 {
                            let s: Pin<Box<dyn Stream<Item = Comment> + Send + 'static>> =
                                Box::pin(stream::once(comment));
                            Ok(s)
                        } else {
                            comment.children().await
                        }
                    })
                    .filter_map(util::ok_log)
                    .flatten(),
            );
            Ok(s as Pin<Box<dyn Stream<Item = Comment> + Send + 'static>>)
        })
    }
}

#[inline]
async fn report_on_comment(
    elem: ArcWebElement,
    basedir: Arc<Path>,
    comment_threshold: i64,
    reply_threshold: i64,
) -> crate::Result<Option<impl Stream<Item = Frame> + Send + 'static>> {
    log::info!("Beginning report_on_comment()");

    let elem = Comment::new(elem, basedir, Default::default()).await?;
    elem.scroll().await?;

    if elem.score().await? < comment_threshold {
        log::info!("Skipping this comment due to its low score");
        return Ok(None);
    }

    // run this code on all the child comments
    // note: we skip the first because it's guaranteed to be the root
    Ok(Some(
        elem.children()
            .await?
            .enumerate()
            .then(|(index, elem)| async move {
                log::info!("Beginning reply #{}", index);
                (elem.score().await, elem)
            })
            .take_while(move |(score, elem)| match score {
                Ok(score) => {
                    if *score >= reply_threshold {
                        true
                    } else {
                        log::info!("Skipping this reply due to its low score");
                        false
                    }
                }
                Err(e) => {
                    log::error!("Unable to get score: {}", e);
                    false
                }
            })
            .then(|elem| async move {
                let f = elem.1.into_frame().await;
                log::info!("Finished reply frame!");
                f
            })
            .filter_map(util::ok_log),
    ))
}

/// Represents the subreddit we've visited.
struct Subreddit {
    driver: WebDriver,
}

impl Subreddit {
    #[inline]
    async fn new(subreddit: &str, net: &str) -> crate::Result<Self> {
        // connect to the server
        let mut caps = DesiredCapabilities::firefox();
        log::info!("Connecting to driver...");
        let driver = WebDriver::new_with_timeout(
            "http://localhost:4444",
            caps,
            Some(Duration::from_secs(10)),
        )
        .await?;
        log::info!("Connected!");

        // visit the subreddit page
        driver
            .get(format!(
                "https://old.reddit.com/r/{}/top/?sort=top&t={}",
                subreddit, net
            ))
            .await?;

        Ok(Self { driver })
    }

    // get the threads in this subreddit's first page
    #[inline]
    async fn threads(self) -> crate::Result<Vec<ThreadHeader>> {
        let driver = Arc::new(self.driver);
        let driver_clone = driver.clone();

        // load up the link elements
        let site_table = driver.find_element(By::Id("siteTable")).await?;
        Ok(site_table
            .find_elements(By::Css(".link:not(promoted)"))
            .await?
            .into_iter()
            .map(move |elem| {
                ThreadHeader::new(ArcWebElement::new(driver_clone.clone(), elem.element_id))
            })
            .collect())
    }
}

struct ThreadHeader {
    elem: ArcWebElement,
}

impl ThreadHeader {
    #[inline]
    fn new(elem: ArcWebElement) -> ThreadHeader {
        ThreadHeader { elem }
    }

    #[inline]
    async fn score(&self) -> crate::Result<i64> {
        let valelem = self
            .elem
            .elem()
            .find_element(By::ClassName("score"))
            .await?;
        let scoreattr = valelem.get_attribute("title").await?;
        match scoreattr {
            None => Err(crate::Error::StaticMsg("Score not found!")),
            Some(score) => {
                let score: i64 = i64::from_str(&score).map_err(|_| crate::Error::NumParseError)?;
                Ok(score)
            }
        }
    }

    #[inline]
    async fn text(&self) -> crate::Result<String> {
        Ok(self
            .elem
            .elem()
            .find_element(By::Tag("a"))
            .await?
            .text()
            .await?)
    }

    #[inline]
    async fn screenshot(&self, basedir: &Path) -> crate::Result<PathBuf> {
        let titlescreenname = basedir.join("title.png");
        util::screenshot_item(self.elem.as_owner(), &self.elem.elem(), &titlescreenname).await?;
        Ok(titlescreenname)
    }

    #[inline]
    async fn into_thread(self) -> crate::Result<RedditThread> {
        log::info!("Going to next page...");
        let attribute = self
            .elem
            .elem()
            .get_attribute("data-url")
            .await?
            .expect("No data-url attribute?");
        let url = format!("https://old.reddit.com{}", attribute);
        let driver = self.elem.as_owner().clone();
        driver.get(url).await?;
        log::info!("Now at next page!");
        Ok(RedditThread { driver })
    }
}

#[derive(Clone)]
struct RedditThread {
    driver: Arc<WebDriver>,
}

impl RedditThread {
    #[inline]
    async fn paragraphs(&self) -> crate::Result<Vec<ArcWebElement>> {
        log::info!("Globbing paragraphs...");
        let driver_clone = self.driver.clone();
        let userpost = self.driver.find_element(By::ClassName("self")).await?;
        Ok(userpost
            .find_elements(By::Tag("p"))
            .await?
            .into_iter()
            .skip(4)
            .map(move |element| ArcWebElement::new(driver_clone.clone(), element.element_id))
            .collect())
    }

    #[inline]
    async fn paragraph_frames(
        &self,
        basedir: Arc<Path>,
    ) -> crate::Result<impl Stream<Item = Frame> + Send + 'static> {
        let paragraphs = self.paragraphs().await?;
        log::info!("Globbed paragraphs!");
        Ok(stream::iter(paragraphs.into_iter())
            .map(move |item| (basedir.clone(), item))
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
                    tts: iteme.inner_html().await.unwrap(),
                    overlaid: String::new(),
                    imagepath: Some(parscreename),
                    imagefadesin: false,
                    persists_after_tts: 1.5,
                }
            }))
    }

    #[inline]
    async fn comment_elements(&self) -> crate::Result<Vec<ArcWebElement>> {
        log::info!("Globbing comments...");
        let driver_clone = self.driver.clone();
        let comments = self
            .driver
            .find_elements(By::Css(".sitetable > div[itemtype]"))
            .await?;
        log::info!("Found comments");
        Ok(comments
            .into_iter()
            .map(move |elem| ArcWebElement::new(driver_clone.clone(), elem.element_id))
            .collect::<Vec<_>>())
    }
}

pub async fn reddit_text_source(
    subreddit: &str,
    upvote_threshold: i64,
    comment_threshold: i64,
    reply_threshold: i64,
    net: &str,
    context: &Context,
) -> crate::Result<impl Stream<Item = Frame> + Send + 'static> {
    let basedir = context.basedir().await;

    // pick out a random thread header
    let sub = Subreddit::new(subreddit, net).await?;
    let items = stream::iter(sub.threads().await?)
        .then(|s| async move { (s.score().await, s) })
        .filter_map(move |(score, s)| {
            if let Ok(score) = score {
                if score >= upvote_threshold {
                    Some(s)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .await;
    let randindex = tls_rng().generate_range(0, items.len());
    let item = items.into_iter().nth(randindex).unwrap();

    let title = item.text().await?;

    context
        .set_thumbnail(
            format!("/r/{} - {}", subreddit, &title),
            "reddit_text.json".to_string(),
        )
        .await;
    context
        .set_video_title(format!("{} - /r/{}", title.to_uppercase(), subreddit))
        .await;

    // take a screenshot of that item and use it as a frame
    let titlescreenname = item.screenshot(&basedir).await?;
    let titleframe = Frame {
        tts: title,
        overlaid: String::new(),
        imagepath: Some(titlescreenname),
        imagefadesin: false,
        persists_after_tts: 1.5,
    };

    // tell driver to go to that index
    let item = item.into_thread().await?;
    let driver = item.driver.clone();

    // within the post, there will be paragraphs, turn each of these into a frame
    let basedir: Arc<Path> = basedir.into_boxed_path().into();
    let parframes = item.paragraph_frames(basedir.clone()).await?;

    // add a frame for the comments
    let comments_frame = Frame {
        tts: "Comments".to_string(),
        overlaid: "Comments".to_string(),
        imagepath: None,
        imagefadesin: false,
        persists_after_tts: 1.5,
    };

    // iterate through the comments and see which ones we want to use
    // note: stream didn't work, doing this now
    let comment_frames = stream::iter(item.comment_elements().await?.into_iter())
        .enumerate()
        .then(move |(index, elem)| {
            log::info!("Beginning processing of top-level comment #{}", index);
            timeout(
                report_on_comment(elem, basedir.clone(), comment_threshold, reply_threshold),
                60,
            )
        })
        .filter_map(|r| {
            log::info!("report_on_comment() finished!");
            match r {
                Ok(Ok(Some(t))) => {
                    log::info!("Finished frame source");
                    Some(t)
                }
                Ok(Ok(None)) => {
                    log::info!("report_on_comment() returned None");
                    None
                }
                Ok(Err(e)) | Err(e) => {
                    log::error!("report_on_comment() error'd: {}", e);
                    None
                }
            }
        })
        .flatten();

    log::info!("Constructing final stream...");

    // we should have all the frames we need, put them together
    let driver_clone = driver.clone();
    let frames = stream::once(titleframe)
        .chain(parframes)
        .chain(stream::once(comments_frame))
        .chain(comment_frames)
        .map(Option::Some)
        .chain(stream::once(()).map(move |_| {
            let dc = driver_clone.clone();
            tokio::spawn(async move { dc.close().await });
            None
        }))
        .filter_map(std::convert::identity);

    log::info!("Stream constructed...");

    // yeah i know this is a cardinal sin but the program hangs if I don't do this
    mem::forget(driver.clone());

    Ok(frames)
}
