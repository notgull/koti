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

mod config;

use crate::context::Context;
use config::YtConfig;
use std::path::PathBuf;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

#[inline]
pub async fn upload_to_youtube(ctx: &Context) -> crate::Result {
    let video_path = ctx.take_video_path().await;
    let thumbnail_path = ctx.take_thumbnail_path().await;
    log::info!(
        "Video path is {:?}, thumbnail path is {:?}",
        &video_path,
        &thumbnail_path
    );

    let config = load_config(ctx).await?;

    log::error!("TODO: upload to youtube");
    Ok(())
}

#[inline]
pub async fn set_token(ctx: &Context, token: String) -> crate::Result {
    let mut cfg = load_config(ctx).await.unwrap_or(Default::default());
    cfg.token = token;
    save_config(ctx, cfg).await
}

#[inline]
async fn youtube_config_path(ctx: &Context) -> PathBuf {
    ctx.datadir().await.join("ytoken.json")
}

#[inline]
async fn load_config(ctx: &Context) -> crate::Result<YtConfig> {
    let mut f = File::open(youtube_config_path(ctx).await).await?;
    let mut data = vec![];
    f.read_to_end(&mut data).await?;
    let y = serde_json::from_slice(&data)?;
    Ok(y)
}

#[inline]
async fn save_config(ctx: &Context, yt: YtConfig) -> crate::Result {
    let data = serde_json::to_vec(&yt)?;
    let mut f = File::create(youtube_config_path(ctx).await).await?;
    f.write_all(&data).await?;
    Ok(())
}
