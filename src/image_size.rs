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

use image::GenericImageView;
use std::path::Path;

#[inline]
pub async fn image_size(img: &Path) -> crate::Result<(u32, u32)> {
    log::info!("Getting image size: {:?}", img);
    let imgpath = img.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut im = image::open(&imgpath)?;
        Ok(im.dimensions())
    })
    .await?
}
