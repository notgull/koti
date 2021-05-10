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

use crate::Context;
use image::{DynamicImage, GenericImageView, Rgba};
use rusttype::{point, vector, Font, Glyph, PositionedGlyph, Scale, ScaledGlyph};
use std::{
    cmp,
    collections::HashMap,
    env, mem,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader},
};

static IMAGE_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn cumulative_width<'a, 'b, I: IntoIterator<Item = &'b ScaledGlyph<'a>>>(
    glyphs: I,
    font: &Font<'_>,
    scale: Scale,
) -> f32
where
    'a: 'b,
{
    glyphs
        .into_iter()
        .scan(None, |last, glyph| {
            let mut w = glyph.h_metrics().advance_width;
            if let Some(last) = last.take() {
                w += font.pair_kerning(scale, last, glyph.id());
            }
            *last = Some(glyph.id());
            Some(w as f32)
        })
        .sum()
}

// load the font data
#[inline]
async fn get_font() -> crate::Result<Font<'static>> {
    let ubuntu_font_path = env::current_dir()?.join("fonts").join("Ubuntu-L.ttf");
    let mut ubuntu_font_info = BufReader::new(File::open(ubuntu_font_path).await?);
    let mut ubuntu_font_data = vec![];
    ubuntu_font_info.read_to_end(&mut ubuntu_font_data).await?;
    Ok(Font::try_from_vec(ubuntu_font_data).expect("Unable to produce font, invalid font file?"))
}

// figure out where the line breaks should go, by iterating over the words, getting the glyphs of the chars
// in those words, summing up the width of each word, and seeing where overflow happens
#[inline]
fn word_glyphs(
    s: String,
    video_size: (usize, usize),
    scale: Scale,
    font: Font<'static>,
) -> Vec<PositionedGlyph<'static>> {
    let v_metrics = font.v_metrics(scale);
    let ascent = v_metrics.ascent;
    let start = point(20.0, 20.0 + ascent);
    let (video_width, video_height) = video_size;

    // get the width of a space
    let space_width = font.glyph(' ').scaled(scale).h_metrics().advance_width;

    s.split(' ')
        .filter(|word| {
            // we don't want empty words
            !word.is_empty()
        })
        .scan((0.0, 0.0), move |&mut (mut x, mut y), word| {
            let scale = scale;

            // from the word, get a vector of glyps
            let glyphs: Vec<_> = font
                .glyphs_for(word.chars())
                // done to satisfy autoref stuff, hopefully should be fixed in a future ver of rusttype
                .map(|glyph| font.glyph(glyph.id()))
                .map(|glyph| glyph.scaled(scale))
                .collect();

            // using the glyphs, figure out the width of the word
            let word_width = cumulative_width(glyphs.iter(), &font, scale);

            // will w + x be greater than the allocated width?
            let (our_x, our_y) = if word_width + x > video_width as f32 {
                // move down a line
                x = word_width;
                y += ascent;
                (0.0, y)
            } else {
                let our_x = x;
                x += word_width + space_width;
                (our_x, y)
            };
            let true_start = start + vector(our_x, our_y);

            // process into glyphs
            Some(
                glyphs
                    .into_iter()
                    .scan((None, 0.0), move |&mut (mut last, mut prev_x), glyph| {
                        if let Some(last) = last {
                            prev_x += glyph.font().pair_kerning(scale, last, glyph.id());
                        }

                        let w = glyph.h_metrics().advance_width;
                        let next = glyph.positioned(true_start + vector(prev_x, 0.0));
                        last = Some(next.id());
                        prev_x += w;
                        Some(next)
                    }),
            )
        })
        .flatten()
        .collect()
}

#[inline]
fn create_border_image<T: image::GenericImageView<Pixel = Rgba<u8>>>(
    above_image: &T,
    border_width: u32,
    color: [u8; 3],
) -> image::RgbaImage {
    image::RgbaImage::from_fn(above_image.width(), above_image.height(), |x, y| {
        // get a subview of the above-image
        let subview = above_image.view(
            x,
            y,
            cmp::min(border_width, above_image.width() - x),
            cmp::min(border_width, above_image.height() - y),
        );

        // iterate over the pixels and get the highest alpha value
        let alpha = subview
            .pixels()
            .map(|(_, _, pix)| pix.0[3])
            .min()
            .unwrap_or(0);
        let [r, g, b] = color;
        image::Rgba([r, b, b, alpha])
    })
}

#[inline]
pub async fn text_overlay(
    s: String,
    ctx: &Context,
    word_color: [u8; 3],
) -> crate::Result<(PathBuf, u32, u32)> {
    // figure out what size of image we need
    let (mut video_width, video_height) = ctx.video_size();
    // add some padding
    let video_width = (video_width - 40) as f32;

    let scale = Scale::uniform(24.0);

    // load the font from a file
    let font = get_font().await?;

    // use the font to create the glyphs
    let glyphs: Vec<_> = word_glyphs(
        s,
        (video_width as usize, video_height as usize),
        scale,
        font,
    );

    let (text_width, text_height) = {
        let min = glyphs
            .first()
            .map(|g| match g.pixel_bounding_box() {
                Some(p) => p.min,
                None => {
                    log::error!("No min found!");
                    panic!("No min found!");
                }
            })
            .unwrap_or_else(|| {
                log::error!("No min_p found!");
                panic!("No min_p found!");
            });
        let max = glyphs
            .last()
            .map(|g| {
                g.pixel_bounding_box()
                    .unwrap_or_else(|| {
                        log::error!("No max found!");
                        panic!("No max found!")
                    })
                    .max
            })
            .unwrap_or_else(|| {
                log::error!("No max_p found!");
                panic!("No max_p found!")
            });
        (max.x - min.x, max.y - min.y)
    };
    let (text_width, text_height) = (text_width as u32, text_height as u32);

    // create the image proper
    let mut img = image::DynamicImage::new_rgba8(text_width + 40, text_height + 40).into_rgba();

    // iterate through the glyphs and draw onto the image
    glyphs.into_iter().for_each(|glyph| {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            glyph.draw(|x, y, intensity| {
                img.put_pixel(
                    x + bounding_box.min.x as u32,
                    y + bounding_box.min.y as u32,
                    image::Rgba([
                        word_color[0],
                        word_color[1],
                        word_color[2],
                        (intensity * 255.0) as u8,
                    ]),
                );
            });
        }
    });

    // create a "border" image underneath with a five-pixel-wide border
    let mut border_img = create_border_image(&img, 5, [0, 0, 0]);

    // overlay one image on top of the other
    image::imageops::overlay(&mut border_img, &img, 0, 0);

    // get the next path
    let path = format!(
        "text_overlay{}.png",
        IMAGE_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let path: PathBuf = ctx.basedir().await.join(path);
    let pathclone = path.clone();

    // save the image
    tokio::task::spawn_blocking(move || {
        image::DynamicImage::ImageRgba8(border_img)
            .save_with_format(pathclone, image::ImageFormat::Png)
    })
    .await??;

    Ok((path, text_width, text_height))
}
