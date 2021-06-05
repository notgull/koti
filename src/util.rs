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

use futures_lite::future;
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    borrow::Cow,
    fmt,
    fs::File,
    future::Future,
    io::{prelude::*, BufReader, BufWriter},
    iter, mem,
    path::Path,
    pin::Pin,
    process::Stdio,
    str::FromStr,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use thirtyfour::{
    common::types::{ElementId, ElementRect},
    prelude::*,
};
use tokio::{process::Command, task::JoinError};

#[inline]
pub async fn cropped_screenshot(
    driver: &WebDriver,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: &Path,
) -> crate::Result {
    log::info!("Screenshotting webdriver");
    driver.screenshot(path).await?;

    let truepath = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // open up the image file
        log::info!("Reading image from file");
        let mut img = image::open(&truepath)?;

        // crop the image
        log::info!("Cropping image");
        let cropped_img = image::imageops::crop(&mut img, x, y, width, height).to_image();

        // save the image
        log::info!("Writing image to file");
        cropped_img.save(truepath)?;
        Result::<(), crate::Error>::Ok(())
    })
    .await??;
    Ok(())
}

#[inline]
pub async fn screenshot_item<'a>(
    driver: &WebDriver,
    element: &WebElement<'a>,
    path: &Path,
) -> crate::Result {
    element.scroll_into_view().await?;
    // get the coordinates of the element
    let ElementRect {
        x,
        y,
        width,
        height,
    } = element.rect().await?;
    let (x, y, width, height) = (
        x.floor() as u32,
        y.floor() as u32,
        width.ceil() as u32,
        height.ceil() as u32,
    );
    log::info!("Element coordinates are ({}, {})", x, y);
    // y is zero because scrolling into view should take care of that
    cropped_screenshot(driver, x, 0, width, height, path).await
    //element.screenshot(path).await?;
    //Ok(())
}

#[derive(Clone)]
pub struct ArcWebElement {
    pub element_id: ElementId,
    pub driver: Arc<WebDriver>,
}

impl ArcWebElement {
    #[inline]
    pub fn new(driver: Arc<WebDriver>, element_id: ElementId) -> Self {
        Self { driver, element_id }
    }

    #[inline]
    pub fn as_owner(&self) -> &Arc<WebDriver> {
        &self.driver
    }

    #[inline]
    pub fn elem(&self) -> WebElement<'_> {
        WebElement::new(&self.driver.session, self.element_id.clone())
    }
}

pin_project_lite::pin_project! {
    pub struct MapFuture<Fut, F> {
        #[pin]
        inner: Fut,
        f: Option<F>,
    }
}

impl<Fut, F> MapFuture<Fut, F> {
    #[inline]
    pub fn new(inner: Fut, f: F) -> Self {
        Self { inner, f: Some(f) }
    }
}

impl<B, Fut: Future, F: FnOnce(Fut::Output) -> B> Future for MapFuture<Fut, F> {
    type Output = B;

    #[inline]
    fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<B> {
        let this = self.project();

        match this.inner.poll(ctx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(g) => {
                Poll::Ready((this.f.take().expect("Future polled after completion"))(g))
            }
        }
    }
}

#[inline]
pub fn ok_log<D, E: fmt::Display>(res: std::result::Result<D, E>) -> Option<D> {
    match res {
        Ok(d) => Some(d),
        Err(e) => {
            log::error!("{}", e);
            None
        }
    }
}

#[inline]
pub fn timeout<Fut: Future>(
    fut: Fut,
    seconds: u64,
) -> impl Future<Output = crate::Result<Fut::Output>> {
    future::or(
        MapFuture::new(fut, Result::Ok),
        MapFuture::new(tokio::time::sleep(Duration::from_secs(seconds)), |()| {
            Err(crate::Error::Timeout)
        }),
    )
}

#[inline]
pub fn cow_str_into_bytes<'a>(cow: Cow<'a, str>) -> Cow<'a, [u8]> {
    match cow {
        Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
        Cow::Owned(s) => Cow::Owned(s.into_bytes()),
    }
}

pin_project_lite::pin_project! {
    #[project = ImmediateOrTaskProjection]
    pub enum ImmediateOrTask<T> {
        Immediate {
            #[pin]
            r: future::Ready<T>
        },
        Task {
            #[pin]
            t: tokio::task::JoinHandle<T>
        },
    }
}

impl<T> From<future::Ready<T>> for ImmediateOrTask<T> {
    #[inline]
    fn from(f: future::Ready<T>) -> Self {
        Self::Immediate { r: f }
    }
}

impl<T> From<tokio::task::JoinHandle<T>> for ImmediateOrTask<T> {
    #[inline]
    fn from(t: tokio::task::JoinHandle<T>) -> Self {
        Self::Task { t }
    }
}

impl<T> Future for ImmediateOrTask<T> {
    type Output = Result<T, JoinError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this {
            ImmediateOrTaskProjection::Immediate { r } => match r.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(t) => Poll::Ready(Ok(t)),
            },
            ImmediateOrTaskProjection::Task { t } => t.poll(cx),
        }
    }
}

#[test]
fn test_timeout() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Unable to construct Tokio runtime")
        .block_on(async move {
            assert_eq!(timeout(future::ready(1u8), 5).await.unwrap(), 1u8);
            assert!(timeout(tokio::time::sleep(Duration::from_secs(11)), 10)
                .await
                .is_err());
        });
}

#[inline]
pub async fn video_length(path: &Path) -> crate::Result<f32> {
    static DURATION_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"Duration: (\d\d):(\d\d):(\d\d).(\d\d)").expect("Regex failed to compile")
    });

    // figure out the length of the sound file using ffmpeg
    let mut c = Command::new("ffmpeg")
        .arg("-i")
        .arg(path)
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    // it is supposed to fail

    let mut textout =
        String::from_utf8(mem::take(&mut c.stdout)).expect("ffmpeg output isn't utf-8?");
    textout.extend(iter::once(
        String::from_utf8(c.stderr).expect("ffmpeg stderr isn't utf-8?"),
    ));
    let total: f32 = match DURATION_REGEX.captures(&textout) {
        Some(caps) => caps
            .iter()
            .skip(1)
            .map(|cap| match cap {
                Some(cap) => usize::from_str(cap.as_str()).expect("Not really an f64?") as f32,
                None => panic!("Should've participated?"),
            })
            .enumerate()
            .fold(0.0, |sum, (index, value)| {
                let multiplier: f32 = match index {
                    0 => 360.0,
                    1 => 60.0,
                    2 => 1.0,
                    3 => 0.01,
                    _ => panic!("More than four captures?"),
                };

                sum + (value * multiplier)
            }),
        None => {
            return Err(crate::Error::StaticMsg(
                "Could not find duration with regex",
            ))
        }
    };

    Ok(total)
}
