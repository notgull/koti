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

use super::{seconds_to_frames, tts::create_tts};
use crate::{
    context::Context,
    frame::Frame,
    image_size::image_size,
    mlt::{Filter, Mlt, PlaylistEntry},
    text2image,
    util::{ImmediateOrTask, MapFuture},
};
use futures_lite::future;
use quick_xml::Writer;
use std::{
    array::IntoIter as ArrayIter,
    iter, mem,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
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
    tts_audio: Option<(PathBuf, f32)>,
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
                static TEXT_OVERLAY_COUNT: AtomicUsize = AtomicUsize::new(0);

                let (video_width, video_height) = ctx.video_size();
                let (t, w, h) = text2image::text_overlay(
                    &overlaid,
                    72.0,
                    video_width as _,
                    video_height as _,
                    [255, 255, 255],
                    [0, 0, 0],
                    6,
                )
                .await?;

                let tpath: PathBuf = ctx.basedir().await.join(format!(
                    "text_overlay{}.png",
                    TEXT_OVERLAY_COUNT.fetch_add(1, Ordering::SeqCst)
                ));
                let tpath = tokio::task::spawn_blocking(move || {
                    t.save_with_format(&tpath, image::ImageFormat::Png)?;
                    crate::Result::Ok(tpath)
                })
                .await??;

                crate::Result::Ok(Some((tpath, w, h)))
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
            tts_audio,
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
    pub fn into_tractor(mut self, mlt: &mut Mlt, ctx: &Context) -> crate::Result<(String, usize)> {
        // three ways of doing this:
        // we just have one image (either the overlay or the direct image)
        // we have both
        let fg_image = self.fg_image.take();
        let text_overlay = self.text_overlay.take();
        let duration = seconds_to_frames(self.duration);

        Ok((
            match (fg_image, text_overlay) {
                (None, None) => panic!("blank frame?"),
                (Some((img, w, h)), None) | (None, Some((img, w, h))) => {
                    self.into_tractor_1_image(mlt, img, w, h, ctx)?
                }
                (Some((img1, w1, h1)), Some((img2, w2, h2))) => {
                    self.into_tractor_2_images(mlt, img1, w1, h1, img2, w2, h2, ctx)?
                }
            },
            duration,
        ))
    }

    #[inline]
    fn into_tractor_1_image(
        mut self,
        mlt: &mut Mlt,
        img: PathBuf,
        w: u32,
        h: u32,
        ctx: &Context,
    ) -> crate::Result<String> {
        // producers:
        //  * the raw image
        //  * the audio
        // playlist:
        //  * one for each
        // tractor:
        //  * multitrack for each playlist
        let (video_width, video_height) = ctx.video_size();
        let (mut vw, mut vh) = (video_width as f32, video_height as f32);
        vw /= 10.0;
        vh /= 10.0;

        let total_duration = seconds_to_frames(self.duration);
        let audio: Option<PathBuf> = self.tts_audio.map(|(t, _)| t);

        // producers
        let producer1 = mlt.add_producer(img).to_string();
        let producer2 = audio.map(|audio| mlt.add_producer(audio).to_string());

        // playlists
        let playlist1 = mlt
            .add_playlist(
                iter::once(PlaylistEntry::Producer {
                    id: producer1,
                    start: 0,
                    end: total_duration,
                }),
                iter::once(Filter::new("resize")),
                /*                iter::once({
                    // use an affine transform to make sure the image fits
                    let w = w as f32;
                    let h = h as f32;
                    let ratio = w / h;
                    let (w, h) = if ratio > (vw / vh) {
                        (vw, h * (vh / vw))
                    } else {
                        // height is 1080, adjust width to fit
                        (w * (vw / vh), vh)
                    };
                    let (x, y) = ((((2.0*vw) - w)/2.0), ((2.0*vh) - h)/2.0);
                    let (x, y, w, h) = (x as u32, y as u32, w as u32, h as u32);

                    Filter::new("affine".to_string())
                        .property("background".to_string(), "colour:0".to_string())
                        .property(
                            "transition.geometry".to_string(),
                            format!("0={} {} {} {}", x, y, w, h),
                        )
                        .property("transition.distort".to_string(), "0".to_string())
                }),*/
                //iter::empty(),
            )
            .to_string();
        let playlist2 = producer2.map(|producer2| {
            mlt.add_playlist(
                iter::once(PlaylistEntry::Producer {
                    id: producer2,
                    start: 0,
                    end: total_duration,
                }),
                //                iter::empty(),
                iter::once(
                    Filter::new("panner".to_string())
                        .property("start".to_string(), "0.5".to_string()),
                ),
            )
            .to_string()
        });

        // tractor
        let tractor = mlt
            .add_tractor(iter::once(playlist1).chain(playlist2), iter::empty())
            .to_string();

        Ok(tractor)
    }

    #[inline]
    fn into_tractor_2_images(
        mut self,
        mlt: &mut Mlt,
        img1: PathBuf,
        w1: u32,
        h1: u32,
        img2: PathBuf,
        w2: u32,
        h2: u32,
        ctx: &Context,
    ) -> crate::Result<String> {
        // producers:
        //  * first image
        //  * second image
        //  * tts audio
        // playlists:
        //  * audio
        //  * one for each image, with a size adjustment filter
        // tractors:
        //  * combines each playlist
        let duration = seconds_to_frames(self.duration);
        let audio = self.tts_audio.map(|(audio, _)| audio);
        let (video_width, video_height) = ctx.video_size();

        // figure out the transform we should do for each image
        // first, figure out the width and height for each image. we get the ratio of each image's height first
        let r1 = (h1 as f32) / ((h1 + h2) as f32);
        let r2 = (h2 as f32) / ((h1 + h2) as f32);

        // multiply the video width and height by these ratios to get the final widths and height for each
        let (vw, vh) = (video_width as f32, video_height as f32);
        let (cw1, ch1) = ((vw * r1) as usize, (vh * r1) as usize);
        let (cw2, ch2) = ((vw * r1) as usize, (vh * r2) as usize);

        // figure out the y coordinates for the images
        let y1 = ((vh - ch2 as f32) / 2.0) as usize;
        let y2 = y1 + ch2;

        // set up the producers
        let img1 = mlt.add_producer(img1).to_string();
        let img2 = mlt.add_producer(img2).to_string();
        let audio = audio.map(|audio| mlt.add_producer(audio).to_string());

        // playlists use an affine transform
        let img1 = mlt
            .add_playlist(
                iter::once(PlaylistEntry::Producer {
                    id: img1,
                    start: 0,
                    end: duration,
                }),
                iter::once(
                    Filter::new("affine".to_string())
                        .property("background".to_string(), "colour:0".to_string())
                        .property(
                            "transition.geometry".to_string(),
                            format!("0=0 {} {} {}", y1, cw1, ch1),
                        )
                        .property("transition.distort".to_string(), "0".to_string()),
                ),
            )
            .to_string();
        let img2 = mlt
            .add_playlist(
                iter::once(PlaylistEntry::Producer {
                    id: img2,
                    start: 0,
                    end: duration,
                }),
                iter::once(
                    Filter::new("affine".to_string())
                        .property("background".to_string(), "colour:0".to_string())
                        .property(
                            "transition.geometry".to_string(),
                            format!("0=0 {} {} {}", y2, cw2, ch2),
                        )
                        .property("transition.distort".to_string(), "0".to_string()),
                ),
            )
            .to_string();
        let audio = audio.map(|audio| {
            mlt.add_playlist(
                iter::once(PlaylistEntry::Producer {
                    id: audio,
                    start: 0,
                    end: duration,
                }),
                iter::empty(),
            )
            .to_string()
        });

        // set up the tractor
        let tractor = mlt
            .add_tractor(ArrayIter::new([img1, img2]).chain(audio), iter::empty())
            .to_string();

        Ok(tractor)
    }
}
