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

use crate::context::Context;
use nanorand::{tls_rng, RNG};
use std::{
    mem,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::{io::AsyncWriteExt, process::Command};

const WORD_SPACE: &str = "8";

static TTS_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn prepare_command(espeak: &mut Command, s: &str, ctx: &Context, basedir: &Path) -> PathBuf {
    // accept input from stdin
    espeak.arg("--stdin");

    // space of 8ms between words
    espeak.args(&["-g", WORD_SPACE]);

    // write sound to the specified output file
    let outpath = format!("tts{}.wav", TTS_COUNTER.fetch_add(1, Ordering::SeqCst));
    let outpath: PathBuf = basedir.join(outpath);

    espeak.arg("-w");
    espeak.arg(&outpath);
    espeak.arg("-m"); // enable html

    // make it loud
    espeak.args(&["-a", "200"]);

    // randomize the pitch
    let pitch = format!("{}", tls_rng().generate_range::<usize>(35, 65));
    espeak.arg("-p");
    espeak.arg(pitch);

    // we pipe in the input, and pipe out the output
    espeak.stdin(Stdio::piped());
    espeak.stdout(Stdio::piped());
    espeak.stderr(Stdio::piped());

    log::info!("Running command:\n\t {:?}", espeak);

    outpath
}

// determined via trial and error
const VALUES_TO_SECONDS: f32 = 4.53425032713551e-05;

// get the duration of a .wav file
async fn wav_duration(path: &Path) -> crate::Result<f32> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        // open up the path in a wavreader and read out the duration
        Ok(hound::WavReader::open(&path)?.duration() as f32 * VALUES_TO_SECONDS)
    })
    .await?
}

#[inline]
pub async fn create_tts(s: &str, ctx: &Context) -> crate::Result<(PathBuf, f32)> {
    // filter out non-ascii characters, we have trouble with them
    let s = s.chars().filter(|c| c.is_ascii()).collect::<String>();

    // we use espeak to create the tts
    let mut espeak = Command::new("espeak");

    let basedir = ctx.basedir().await;

    // configure the command
    let outpath = prepare_command(&mut espeak, &s, ctx, &basedir);

    // spawn the child process
    let mut child = espeak.spawn()?;

    // pipe the string into stdin, and then drop it
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| crate::Error::StaticMsg("stdin already taken?"))?;
    stdin.write_all(s.as_bytes()).await?;
    stdin.flush().await?;
    mem::drop(stdin);

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
