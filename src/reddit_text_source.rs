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
use once_cell::sync::OnceCell;
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
    time::Duration,
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
    async fn score(&self) -> crate::Result<i64> {
        let score = self
            .commentbody
            .elem()
            .find_element(By::ClassName("score"))
            .await?;
        match score.get_attribute("title").await? {
            None => Err(crate::Error::StaticMsg(
                "Score doesn't exist, likely a blunder",
            )),
            Some(score) => i64::from_str(&score).map_err(|_| crate::Error::NumParseError),
        }
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
                    } => (x, y, cmp::max(width, widthtop), height + heighttop + 5),
                };
                res.1 = 0;
                self.screenshot_rect.set(res).ok();
                Ok(res)
            }
        }
    }

    #[inline]
    async fn screenshot(&self) -> crate::Result<PathBuf> {
        let path = self.save_basedir.join(&format!(
            "comment{}.png",
            GLOBAL_NUMBER.fetch_add(1, Ordering::Relaxed)
        ));
        let (x, y, width, height) = self.screenshot_rect().await?;
        util::cropped_screenshot(self.elem.as_owner(), x, y, width, height, &path).await?;
        Ok(path)
    }

    #[inline]
    async fn text(&self) -> crate::Result<String> {
        Ok(stream::iter(
            self.commentbody
                .elem()
                .find_elements(By::Tag("p"))
                .await?
                .into_iter()
                .skip(1usize),
        )
        .then(|e| async move { e.text().await })
        .filter_map(Result::ok)
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
        Ok(Frame {
            tts: text,
            overlaid: String::new(),
            imagepath: Some(sspath),
            imagefadesin: false,
            persists_after_tts: 1.5,
        })
    }

    #[inline]
    async fn into_frame(self) -> crate::Result<Frame> {
        self.frame().await
    }

    #[inline]
    async fn direct_children(self) -> crate::Result<impl Stream<Item = Comment> + Send + 'static> {
        let driver_clone = self.elem.as_owner().clone();
        let basedir_clone = self.save_basedir.clone();
        let (x, y, width, height) = self.screenshot_rect().await?;
        let direct_children = self
            .elem
            .elem()
            .find_elements(By::XPath(
                "/div[contains(@class, 'child')]/div[contains(@class, 'comment')]",
            ))
            .await?;
        let direct_children = direct_children
            .into_iter()
            .map(move |elem| {
                (
                    ArcWebElement::new(driver_clone.clone(), elem.element_id),
                    basedir_clone.clone(),
                )
            })
            .collect::<Vec<_>>();

        Ok(stream::once(self).chain(
            stream::iter(direct_children.into_iter())
                .then(move |(elem, basedir)| {
                    Comment::new(
                        elem,
                        basedir,
                        CommentStatus::LowerLevel {
                            x,
                            y,
                            width,
                            height,
                        },
                    )
                })
                .filter_map(Result::ok),
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
                    .then(|comment| comment.children())
                    .filter_map(Result::ok)
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
    let elem = Comment::new(elem, basedir, Default::default()).await?;
    elem.scroll().await?;

    if elem.score().await? < comment_threshold {
        return Ok(None);
    }

    // run this code on all the child comments
    // note: we skip the first because it's guaranteed to be the root
    Ok(Some(
        elem.children()
            .await?
            .then(|elem| async move { (elem.score().await, elem) })
            .filter_map(move |(score, elem)| {
                if let Ok(score) = score {
                    if score >= reply_threshold {
                        Some(elem)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .then(|elem| elem.into_frame())
            .filter_map(Result::ok),
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
        println!("Connecting to driver...");
        let driver = WebDriver::new_with_timeout(
            "http://localhost:4444",
            caps,
            Some(Duration::from_secs(10)),
        )
        .await?;
        println!("Connected!");

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
            .map(move |element| ArcWebElement::new(driver_clone.clone(), element.element_id))
            .collect())
    }

    #[inline]
    async fn paragraph_frames(
        &self,
        basedir: Arc<Path>,
    ) -> crate::Result<impl Stream<Item = Frame> + Send + 'static> {
        Ok(stream::iter(self.paragraphs().await?.into_iter())
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
                    tts: iteme.text().await.unwrap(),
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
        Ok(driver_clone
            .clone()
            .find_element(By::ClassName("sitetable"))
            .await?
            .find_elements(By::XPath("/div[contains(@class, 'comment')]"))
            .await?
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
            format!("/r/{}: {}", subreddit, &title),
            Path::new("reddit_text.json").to_path_buf(),
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
    let comment_frames = stream::iter(item.comment_elements().await?.into_iter())
        .then(move |elem| {
            report_on_comment(elem, basedir.clone(), comment_threshold, reply_threshold)
        })
        .filter_map(|r| if let Ok(Some(t)) = r { Some(t) } else { None })
        .flatten();

    // we should have all the frames we need, put them together
    Ok(stream::once(titleframe)
        .chain(parframes)
        .chain(stream::once(comments_frame))
        .chain(comment_frames))
}
