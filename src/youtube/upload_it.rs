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

use super::YtConfig;
use std::iter;

const ROOT_URL: &'static str = "https://youtube.googleapis.com/";
const UPLOAD_APPEND: &'static str = "upload/youtube/v3/videos";

#[inline]
pub async fn upload_video(cfg: YtConfig, vidpath: &Path, thumbpath: &Path, title: String, desc: String) -> crate::Result {
    let url = format!("{}{}", ROOT_URL, UPLOAD_APPEND);
    let params = 
}