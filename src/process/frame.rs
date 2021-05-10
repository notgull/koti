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

use super::{overlay::text_overlay, tts::create_tts};
use crate::{
    context::Context,
    frame::Frame,
    image_size::image_size,
    util::{ImmediateOrTask, MapFuture},
};
use futures_lite::future;
use quick_xml::Writer;
use std::{path::PathBuf, sync::Arc};
use tokio::process::Command;

#[inline]
fn emptied(s: String) -> Option<String> {
    match s.is_empty() {
        true => None,
        false => Some(s),
    }
}

#[derive(Debug)]
pub struct ConvertedFrame {
    fg_image: Option<(PathBuf, u32, u32)>,
    fades_in_after: Option<f32>,
    text_overlay: Option<(PathBuf, u32, u32)>,
    tts_audio: Option<PathBuf>,
    duration: f32,
}

impl ConvertedFrame {
    #[inline]
    pub async fn from_frame(frame: Frame, ctx: Arc<Context>) -> crate::Result<Self> {
        log::info!("Converting frame to finalized portion: {:?}", frame);

        let Frame {
            tts,
            overlaid,
            imagepath,
            imagefadesin,
            persists_after_tts,
        } = frame;

        // tts .wav file
        let ctx_clone = ctx.clone();
        let tts_audio: ImmediateOrTask<_> = match emptied(tts) {
            Some(tts) => tokio::spawn(async move {
                let (t, duration) = create_tts(&tts, &ctx_clone).await?;
                crate::Result::Ok(Some((t, duration)))
            })
            .into(),
            None => future::ready(Ok(None)).into(),
        };

        // text overlay image file
        let text_overlay: ImmediateOrTask<_> = match emptied(overlaid) {
            Some(overlaid) => tokio::spawn(async move {
                let (t, w, h) = text_overlay(overlaid, &ctx, [255, 255, 255]).await?;
                crate::Result::Ok(Some((t, w, h)))
            })
            .into(),
            None => future::ready(Ok(None)).into(),
        };

        // image in the foreground
        let fg_image: ImmediateOrTask<_> = match imagepath {
            Some(imagepath) => tokio::spawn(async move {
                let (w, h) = image_size(&imagepath).await?;
                crate::Result::Ok(Some((imagepath, w, h)))
            })
            .into(),
            None => future::ready(Ok(None)).into(),
        };

        // combine all the tasks
        let ((tts_audio, text_overlay_path), fg_image) =
            future::zip(future::zip(tts_audio, text_overlay), fg_image).await;
        let tts_audio = tts_audio??;
        let duration = if let Some((_, ref duration)) = tts_audio {
            *duration
        } else {
            0.0
        };
        let text_overlay = text_overlay_path??;
        let fg_image = fg_image??;
        log::info!("Finished converting frame");

        Ok(ConvertedFrame {
            fg_image,
            tts_audio: tts_audio.map(|(t, _)| t),
            text_overlay,
            fades_in_after: if imagefadesin { Some(1.0) } else { None },
            duration: persists_after_tts + duration,
        })
    }

    #[inline]
    pub fn duration(&self) -> f32 {
        self.duration
    }

    #[inline]
    pub fn add_frame_to_melt<T: std::io::Write>(self, outxml: &mut Writer<T>) -> crate::Result {
        Ok(())
    }
}
