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
use nanorand::{tls_rng, RNG};
use std::{
    io::ErrorKind,
    mem,
    path::{Path, PathBuf},
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
};

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Music {
    entries: Vec<MusicEntry>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct MusicEntry {
    name: String,
    path: PathBuf,
    attribution: String,
}

impl Music {
    #[inline]
    pub async fn load(ctx: &Context) -> crate::Result<Self> {
        let mut jsonfile = match File::open(jsonpath(ctx).await).await {
            Ok(j) => j,
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::PermissionDenied) => {
                return Ok(Default::default());
            }
            Err(e) => return Err(e.into()),
        };
        let mut jsondata = String::new();
        jsonfile.read_to_string(&mut jsondata).await?;
        mem::drop(jsonfile);

        // prase json data
        let m: Self = serde_json::from_str(&jsondata)?;
        Ok(m)
    }

    #[inline]
    pub async fn save(&self, ctx: &Context) -> crate::Result {
        let mut jsonfile = File::create(jsonpath(ctx).await).await?;
        let jsondata = serde_json::to_vec(self)?;
        jsonfile.write_all(&jsondata).await?;
        Ok(())
    }

    #[inline]
    pub fn add_track(&mut self, name: String, path: PathBuf, attribution: String) {
        self.entries.push(MusicEntry {
            name,
            path,
            attribution,
        });
    }

    /// Returns path to music and attribution string.
    #[inline]
    pub fn random_track(&self) -> (&Path, &str) {
        let entry = &self.entries[tls_rng().generate_range(0, self.entries.len())];
        (&entry.path, &entry.attribution)
    }
}

#[inline]
async fn jsonpath(ctx: &Context) -> PathBuf {
    ctx.datadir().await.join("music.json")
}
