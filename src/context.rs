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

use event_listener::{Event, EventListener};
use std::{mem, path::PathBuf};
use tokio::sync::Mutex;

#[derive(Debug, Default)]
struct ContextCore {
    thumbnail_template: Option<PathBuf>,
    thumbnail_text: Option<String>,
    video_title: Option<String>,
    basedir: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct Context {
    core: Mutex<ContextCore>,
    thumbnail_ready: Event,
}

impl Context {
    #[inline]
    pub const fn new() -> Self {
        Context {
            core: Mutex::const_new(ContextCore {
                thumbnail_template: None,
                thumbnail_text: None,
                video_title: None,
                basedir: None,
            }),
            thumbnail_ready: Event::new(),
        }
    }

    #[inline]
    pub async fn set_basedir(&self, basedir: PathBuf) {
        if mem::replace(&mut self.core.lock().await.basedir, Some(basedir)).is_some() {
            panic!("Basedir already exists!");
        }
    }

    #[inline]
    pub async fn basedir(&self) -> PathBuf {
        self.core.lock().await.basedir.clone().unwrap()
    }

    #[inline]
    pub async fn reset(&self) {
        *self.core.lock().await = Default::default();
    }

    #[inline]
    pub async fn set_video_title(&self, title: String) {
        let mut core = self.core.lock().await;
        if mem::replace(&mut core.video_title, Some(title)).is_some() {
            panic!("Video title already exists!");
        }
    }

    #[inline]
    pub async fn take_video_title(&self) -> String {
        self.core
            .lock()
            .await
            .video_title
            .take()
            .expect("Video title already taken!")
    }

    #[inline]
    pub async fn set_thumbnail(&self, text: String, template: PathBuf) {
        let mut core = self.core.lock().await;
        if mem::replace(&mut core.thumbnail_text, Some(text)).is_some() {
            panic!("Thumbnail text already exists!");
        }
        if mem::replace(&mut core.thumbnail_template, Some(template)).is_some() {
            panic!("Thumbnail template already exists!");
        }

        mem::drop(core);
        self.thumbnail_ready.notify_additional(usize::MAX);
    }

    #[inline]
    pub async fn take_thumbnail_text(&self) -> String {
        self.core
            .lock()
            .await
            .thumbnail_text
            .take()
            .expect("Thumbnail text already taken!")
    }

    #[inline]
    pub async fn take_thumbnail_template(&self) -> PathBuf {
        self.core
            .lock()
            .await
            .thumbnail_template
            .take()
            .expect("Thumbnail template already taken!")
    }

    #[inline]
    pub async fn wait_for_thumbnail(&self) {
        self.thumbnail_ready.listen().await
    }
}
