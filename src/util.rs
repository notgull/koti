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

use std::{fs::File as SyncFile, mem, path::Path, pin::Pin, sync::Arc};
use thirtyfour::{
    common::types::{ElementId, ElementRect},
    prelude::*,
};
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

#[inline]
pub async fn cropped_screenshot(
    driver: &WebDriver,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    path: &Path,
) -> crate::Result {
    driver.screenshot(path).await?;

    // open up the image file
    let mut v = vec![];
    let mut f = File::open(path).await?;
    f.read_buf(&mut v).await?;
    let mut img = image::load_from_memory(&v)?;

    // crop the image
    let cropped_img = image::imageops::crop(&mut img, x, y, width, height);

    // save the image
    v.clear();
    img.write_to(&mut v, image::ImageOutputFormat::Png)?;
    let mut f = File::create(path).await?;
    f.write_all(&v).await?;
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
    cropped_screenshot(driver, x, y, width, height, path).await
}

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
