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

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use rusttype::{point, vector, Font, PositionedGlyph, Scale, ScaledGlyph};
use std::{cmp, env, mem};
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader},
    sync::OnceCell,
};

const TEXT_MARGIN: f32 = 0.7;
const IMAGE_MARGIN: f32 = 1.6;

static FONT: OnceCell<Font<'static>> = OnceCell::const_new();

#[inline]
async fn load_font() -> crate::Result<&'static Font<'static>> {
    FONT.get_or_try_init(|| async {
        log::info!("Loading font from file");
        let fontpath = env::current_dir()?.join("fonts").join("Ubuntu-L.ttf");
        let mut fontfile = BufReader::new(File::open(fontpath).await?);
        let mut fontdata = vec![];
        fontfile.read_to_end(&mut fontdata).await?;

        let font = Font::try_from_vec(fontdata).expect("Invalid font file?");
        Ok(font)
    })
    .await
}

// get the total width of some scaled glyphs
#[inline]
fn cumulative_width<'a, 'b, I: IntoIterator<Item = &'a ScaledGlyph<'b>>>(
    glyphs: I,
    scale: Scale,
) -> f32
where
    'b: 'a,
{
    glyphs
        .into_iter()
        .scan(None, |last, glyph| {
            let mut w = glyph.h_metrics().advance_width;
            if let Some(last) = last.take() {
                w += glyph.font().pair_kerning(scale, last, glyph.id());
            }
            *last = Some(glyph.id());
            Some(w as f32)
        })
        .sum()
}

#[inline]
fn word_glyphs(
    s: &str,
    scale: Scale,
    max_image_width: u32,
    max_image_height: u32,
    font: &'static Font<'static>,
) -> crate::Result<Vec<PositionedGlyph<'static>>> {
    let v_metrics = font.v_metrics(scale);
    let ascent = v_metrics.ascent;
    let start = point(TEXT_MARGIN * scale.x, TEXT_MARGIN * scale.y);

    // get the width of a space
    let space_width = font.glyph(' ').scaled(scale).h_metrics().advance_width;

    let mut overflow = false;
    let (max_image_width, max_image_height) = (max_image_width as f32, max_image_height as f32);

    // iterate by word (i.e. split by space)
    let glyphs: Vec<_> = s
        .split(' ')
        .filter(|word| {
            // filter out empty words
            !word.is_empty()
        })
        .scan((0.0, 0.0), move |&mut (ref mut x, ref mut y), word| {
            let scale = scale;

            if overflow {
                return None;
            }

            // from the word, get the necessary glyphs
            let glyphs: Vec<_> = font
                .glyphs_for(word.chars())
                // rusttype bug workaround
                .map(|glyph| font.glyph(glyph.id()))
                .map(|glyph| glyph.scaled(scale))
                .collect();

            // get the width of the word
            let word_width = cumulative_width(glyphs.iter(), scale);

            // figure out whether we'll go over the line with
            let (word_x, word_y) = if word_width + *x > max_image_width as f32 {
                *x = word_width;
                *y += v_metrics.line_gap + v_metrics.ascent;
                if *y > max_image_height {
                    overflow = true;
                    return None;
                }
                (0.0, *y)
            } else {
                let word_x = *x;
                *x += word_width + space_width;
                (word_x, *y)
            };

            let true_start = start + vector(word_x, word_y);

            Some(glyphs.into_iter().scan(
                (None, 0.0),
                move |&mut (ref mut last, ref mut prev_x), glyph| {
                    if let Some(last) = last.take() {
                        *prev_x += glyph.font().pair_kerning(scale, last, glyph.id());
                    }

                    let w = glyph.h_metrics().advance_width;
                    log::debug!("glyph advance width is {}", w);
                    let pt = true_start + vector(*prev_x, 0.0);
                    log::debug!("Putting glyph at {:?}", pt);
                    let next = glyph.positioned(pt);
                    *last = Some(next.id());
                    *prev_x += w;
                    log::debug!("New x is: {}", prev_x);
                    Some(next)
                },
            ))
        })
        .flatten()
        .collect();

    if overflow {
        Err(crate::Error::GlyphOverflow)
    } else {
        Ok(glyphs)
    }
}

/// Generate an image to use as a border background.
#[inline]
fn border_image<I: GenericImageView<Pixel = Rgba<u8>>>(
    i: &I,
    border_width: u32,
    border_color: [u8; 3],
) -> RgbaImage {
    let border_radius = border_width / 2;
    let (iwidth, iheight) = i.dimensions();
    RgbaImage::from_fn(iwidth, iheight, |x, y| {
        let scanx = x.saturating_sub(border_radius);
        let scany = y.saturating_sub(border_radius);
        let scanw = cmp::min(border_width, iwidth.saturating_sub(scanx));
        let scanh = cmp::min(border_width, iheight.saturating_sub(scany));
        //        log::debug!("Scanning for rectangle: ({}, {}, {}, {})", scanx, scany, scanw, scanh);

        let alpha = if let (0, _) | (_, 0) = (scanw, scanh) {
            0
        } else {
            let view = i.view(scanx, scany, scanw, scanh);

            view.pixels()
                .map(|(_, _, Rgba([_, _, _, a]))| a)
                .max()
                .unwrap_or(0)
        };
        let [r, g, b] = border_color;
        Rgba([r, g, b, alpha])
    })
}

/// Given a string of text, a font size, and a maximum image size, create an image containing text.
#[inline]
pub async fn text_overlay(
    text: &str,
    font_size: f32,
    max_image_width: u32,
    max_image_height: u32,
    word_color: [u8; 3],
    border_color: [u8; 3],
    border_width: u32,
) -> crate::Result<(RgbaImage, u32, u32)> {
    let font = load_font().await?;
    let scale = Scale::uniform(font_size);

    // set up the glyphs
    let glyphs = word_glyphs(text, scale, max_image_width, max_image_height, font)?;

    // calculate the sizes needed for the text
    let (text_width, text_height) = {
        let min = glyphs
            .first()
            .map(|g| g.pixel_bounding_box().expect("No min in bounding box?").min)
            .expect("Empty string?");
        let bottommost = glyphs
            .last()
            .map(|g| g.pixel_bounding_box().expect("No max in bounding box?").max)
            .expect("Empty string?");
        let leftmost = glyphs
            .iter()
            .filter_map(|g| g.pixel_bounding_box().map(|p| p.max))
            .max_by(|m1, m2| m1.x.cmp(&m2.x))
            .expect("Empty string?");
        (leftmost.x - min.x, bottommost.y - min.y)
    };

    let (text_width, text_height) = (text_width as u32, text_height as u32);
    let margin: u32 = (IMAGE_MARGIN * font_size) as u32;
    let (image_width, image_height) = (text_width + margin, text_height + margin);

    // create an image
    let mut text_img = RgbaImage::new(image_width, image_height);

    // draw the glyphs onto the image
    glyphs.into_iter().for_each(|glyph| {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            let bbx = bounding_box.min.x as u32;
            let bby = bounding_box.min.y as u32;
            glyph.draw(|x, y, intensity| {
                //                log::debug!("Putting pixel at ({}, {}) (relative to {}, {})", x, y, bbx, bby);
                text_img.put_pixel(
                    x + bbx,
                    y + bby,
                    Rgba([
                        word_color[0],
                        word_color[1],
                        word_color[2],
                        (intensity * 255.0) as u8,
                    ]),
                );
            });
        }
    });

    // create a border image
    let mut border_img = border_image(&text_img, border_width, border_color);

    // merge the images
    image::imageops::overlay(&mut border_img, &text_img, 0, 0);

    Ok((border_img, image_width, image_height))
}
