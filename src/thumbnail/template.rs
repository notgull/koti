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

use crate::{context::Context, text2image};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Copy, Clone, serde::Deserialize, serde::Serialize)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ThumbnailTemplate {
    id: String,
    path: PathBuf,
    trect: Rect,
}

impl ThumbnailTemplate {
    #[inline]
    pub fn new(id: String, path: PathBuf, x: u32, y: u32, w: u32, h: u32) -> Self {
        Self {
            id,
            path,
            trect: Rect { x, y, w, h },
        }
    }

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub async fn apply(self, text: String, ctx: Arc<Context>) -> crate::Result<PathBuf> {
        let Self { id, path, trect } = self;
        let outpath = ctx.basedir().await.join("thumbnail.png");
        let base_image = tokio::task::spawn_blocking(move || {
            let mut base_image = image::open(&path)?;
            crate::Result::Ok(base_image)
        });

        // iterate over the text image until we can effectively fit it within the rect
        let timage = tokio::spawn(async move {
            let mut current_size = 80f32;
            let timage = loop {
                let res = text2image::text_overlay(
                    &text,
                    current_size,
                    trect.w,
                    trect.h,
                    [255, 255, 255],
                    [15, 15, 15],
                    5,
                )
                .await;
                match res {
                    Ok((timage, _, _)) => break timage,
                    Err(crate::Error::GlyphOverflow) => {
                        current_size -= 1.0;
                        if current_size < 1.0f32 {
                            return Err(crate::Error::GlyphOverflow);
                        }
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            };

            crate::Result::Ok(timage)
        });

        let (mut base_image, timage) = (base_image.await??, timage.await??);

        image::imageops::overlay(&mut base_image, &timage, trect.x, trect.y);

        // save the image
        tokio::task::spawn_blocking(move || {
            base_image.save_with_format(&outpath, image::ImageFormat::Png)?;
            crate::Result::Ok(outpath)
        })
        .await?
    }
}
