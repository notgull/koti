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

use crate::{context::Context, util::video_length};
use nanorand::{tls_rng, RNG};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    mem,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::{fs::File, io::AsyncWriteExt, process::Command};

const WORD_SPACE: &str = "4";

static TTS_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn prepare_command(text2wave: &mut Command, source: PathBuf, ctx: &Context) -> PathBuf {
    // figure out the output path
    let mut outpath = source
        .parent()
        .expect("not physically possible #2")
        .join(source.file_stem().expect("not physically possible"));
    outpath.set_extension("wav");

    // use the better voice type
    text2wave.arg("-eval");
    text2wave.arg("(voice_cmu_us_slt_arctic_hts)\n(Parameter.set 'Duration_Strech 0.75)");

    text2wave.arg(source);

    text2wave.arg("-o");
    text2wave.arg(&outpath);

    log::info!("Running TTS command: {:?}", text2wave);

    outpath
}

// get rid of everything in between brackets
static BRACKETS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"</?[\w\d\s]*>").expect("Regex failed to compile"));

// determined via trial and error
const VALUES_TO_SECONDS: f32 = 4.53425032713551e-05;

// get the duration of a .wav file
async fn wav_duration(path: &Path) -> crate::Result<f32> {
    /*let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // open up the path in a wavreader and read out the duration
        Ok(hound::WavReader::open(&path)?.duration() as f32 * VALUES_TO_SECONDS)
    })
    .await?*/
    video_length(path).await
}

#[inline]
pub async fn create_tts(s: &str, ctx: &Context) -> crate::Result<(PathBuf, f32)> {
    // filter out non-ascii characters, we have trouble with them
    let s = s.chars().filter(|c| c.is_ascii()).collect::<String>();

    let s = BRACKETS.replace(&s, "").into_owned();

    // write to a text file
    let basedir = ctx.basedir().await;
    let source = basedir.join(format!(
        "tts{}.txt",
        TTS_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let mut f = File::create(&source).await?;
    f.write_all(s.as_bytes()).await?;
    mem::drop(f);

    // we use text2wave to create the tts
    let mut t2w = Command::new("text2wave");

    // configure the command
    let outpath = prepare_command(&mut t2w, source, ctx);

    // spawn the child process
    let mut child = t2w.spawn()?;

    // wait for the command to go
    let Output {
        status,
        stdout,
        stderr,
    } = child.wait_with_output().await?;
    let stderr = String::from_utf8(stderr).ok();

    // if the exit status is bad, error out
    if !status.success() {
        return Err(crate::Error::EspeakError(status.code(), stderr));
    }

    // output the stderr and stdout
    match stderr {
        Some(stderr) if stderr.is_empty() => (),
        Some(stderr) => {
            log::error!("Espeak stderr: {}", stderr);
        }
        None => {
            log::error!("Espeak stderr: <not utf-8>");
        }
    }

    match String::from_utf8(stdout) {
        Ok(stdout) if stdout.is_empty() => (),
        Ok(stdout) => {
            log::warn!("Espeak stdout: {}", stdout);
        }
        Err(_) => {
            log::warn!("Espeak stdout: <not utf-8>");
        }
    }

    // get the duration of the file
    let duration = wav_duration(&outpath).await?;

    Ok((outpath, duration))
}
