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

mod template;

use crate::context::Context;
use std::{path::PathBuf, sync::Arc};
use template::ThumbnailTemplate;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

#[inline]
pub async fn create_thumbnail(ctx: Arc<Context>) -> crate::Result {
    // load thumbnail templates
    let thumbnails = load_thumbnails(&ctx).await?;

    log::info!("Waiting for thumbnail info");
    ctx.wait_for_thumbnail().await;
    log::info!("Thumbnail wait ended!");

    // get the thumbnail stuff
    let text = ctx.take_thumbnail_text().await;
    let template = ctx.take_thumbnail_template().await;

    // get the desired template
    let template = thumbnails
        .into_iter()
        .find(|thumbnail| thumbnail.id() == &template)
        .ok_or_else(|| {
            crate::Error::Msg(format!("Unable to find template with name: {}", template))
        })?;

    // apply it
    let applied_path = template.apply(text, ctx.clone()).await?;

    ctx.set_thumbnail_path(applied_path).await;

    Ok(())
}

#[inline]
pub async fn add_thumbnail_to_collection(
    ctx: &Context,
    id: String,
    path: PathBuf,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> crate::Result {
    let mut thumbnails = load_thumbnails(ctx).await.unwrap_or(vec![]);
    thumbnails.push(ThumbnailTemplate::new(id, path, x, y, w, h));
    save_thumbnails(ctx, thumbnails).await
}

#[inline]
async fn thumbnails_path(ctx: &Context) -> PathBuf {
    ctx.datadir().await.join("thumbnails.json")
}

#[inline]
async fn load_thumbnails(ctx: &Context) -> crate::Result<Vec<ThumbnailTemplate>> {
    let mut f = File::open(thumbnails_path(ctx).await).await?;
    let mut data = vec![];
    f.read_to_end(&mut data).await?;

    let d = serde_json::from_slice(&data)?;
    Ok(d)
}

#[inline]
async fn save_thumbnails(ctx: &Context, t: Vec<ThumbnailTemplate>) -> crate::Result {
    let mut f = File::create(thumbnails_path(ctx).await).await?;
    let data = serde_json::to_vec(&t)?;
    f.write_all(&data).await?;
    Ok(())
}
