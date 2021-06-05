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
use google_youtube3::{
    api::{Video, VideoSnippet, VideoStatus},
    YouTube,
};
use std::path::PathBuf;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

#[inline]
pub async fn upload_video(
    ctx: &Context,
    video_path: PathBuf,
    thumbnail_path: PathBuf,
    mut video_title: String,
    video_desc: String,
) -> crate::Result {
    let config = load_config(ctx).await?;
    let YtConfig {
        client_id,
        client_secret,
    } = config;

    // sanitize the title
    let video_title: String = video_title.chars().filter(|c| c.is_ascii()).collect();
    log::info!("Video title: {}", &video_title);
    if video_title.is_empty() {
        return Err(crate::Error::StaticMsg("Video title was empty!"));
    }

    // create the oauth authorization
    let secret = yup_oauth2::ApplicationSecret {
        client_id,
        client_secret,
        token_uri: "https://accounts.google.com/o/oauth2/token".to_string(),
        auth_uri: "https://accounts.google.com/o/oauth2/auth".to_string(),
        ..Default::default()
    };
    let auth = yup_oauth2::InstalledFlowAuthenticator::builder(
        secret,
        yup_oauth2::InstalledFlowReturnMethod::HTTPRedirect,
    )
    .persist_tokens_to_disk("tokencache.json")
    .build()
    .await?;

    log::info!("Acquired the authentication token for YouTube");

    // open up the YouTube API
    let mut yt = YouTube::new(
        hyper::Client::builder().build(hyper_rustls::HttpsConnector::with_native_roots()),
        auth,
    );

    // upload the video
    let mut req = Video::default();
    req.snippet = Some(VideoSnippet {
        title: Some(video_title),
        description: Some(video_desc),
        ..Default::default()
    });
    req.status = Some(VideoStatus {
        privacy_status: Some("public".to_string()),
        ..Default::default()
    });

    let (_, video) = yt
        .videos()
        .insert(req)
        .upload_resumable(
            File::open(video_path).await?.into_std().await,
            "video/webm".parse().unwrap(),
        )
        .await
        .expect("Failed to upload to YouTube");

    log::debug!("Video is: {:?}", &video);

    // upload the thumbnail
    log::info!("Video has been uploaded, uploading thumbnail...");
    yt.thumbnails()
        .set(video.id.expect("Video has no id?").as_str())
        .upload_resumable(
            File::open(thumbnail_path).await?.into_std().await,
            "image/png".parse().unwrap(),
        )
        .await
        .expect("Failed to set thumbnail for video");

    log::info!("Should now be uploaded and processing on YouTube!");
    Ok(())
}

#[inline]
pub async fn upload_to_youtube(ctx: &Context) -> crate::Result {
    let video_path = ctx.take_video_path().await;
    let thumbnail_path = ctx.take_thumbnail_path().await;
    let video_title = ctx.take_video_title().await;
    let video_desc = ctx.take_video_description().await;
    log::info!(
        "Video path is {:?}, thumbnail path is {:?}",
        &video_path,
        &thumbnail_path
    );

    upload_video(ctx, video_path, thumbnail_path, video_title, video_desc).await
}

#[inline]
pub async fn set_token(ctx: &Context, cid: String, cs: String) -> crate::Result {
    let mut cfg = load_config(ctx).await.unwrap_or(Default::default());
    cfg.client_id = cid;
    cfg.client_secret = cs;
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
