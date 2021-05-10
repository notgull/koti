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

use image::ImageError;
use quick_xml::Error as XmlError;
use std::{fmt, io::Error as IoError};
use thirtyfour::error::WebDriverError;
use tokio::task::JoinError;

#[derive(Debug)]
pub enum Error {
    Msg(String),
    StaticMsg(&'static str),
    Io(IoError),
    Selenium(WebDriverError),
    Image(ImageError),
    Join(JoinError),
    NumParseError,
    ScoreTimedOut,
    Timeout,
    EspeakError(Option<i32>, Option<String>),
    Hound(hound::Error),
    Xml(XmlError),
}

impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Msg(s) => f.write_str(s),
            Self::StaticMsg(s) => f.write_str(s),
            Self::Io(i) => fmt::Display::fmt(i, f),
            Self::Selenium(w) => fmt::Display::fmt(w, f),
            Self::Image(i) => fmt::Display::fmt(i, f),
            Self::Join(j) => fmt::Display::fmt(j, f),
            Self::NumParseError => f.write_str("Could not parse number"),
            Self::ScoreTimedOut => f.write_str("Score timed out"),
            Self::Timeout => f.write_str("Operation timed out"),
            Self::EspeakError(None, None) => f.write_str("Espeak failed"),
            Self::EspeakError(Some(code), None) => write!(f, "Espeak failed with code {}", code),
            Self::EspeakError(None, Some(err)) => write!(f, "Espeak failed: {}", err),
            Self::EspeakError(Some(code), Some(err)) => {
                write!(f, "Espeak failed with code {}: {}", code, err)
            }
            Self::Hound(h) => fmt::Display::fmt(h, f),
            Self::Xml(x) => fmt::Display::fmt(x, f),
        }
    }
}

impl From<IoError> for Error {
    #[inline]
    fn from(i: IoError) -> Error {
        Self::Io(i)
    }
}

impl From<WebDriverError> for Error {
    #[inline]
    fn from(w: WebDriverError) -> Error {
        Self::Selenium(w)
    }
}

impl From<ImageError> for Error {
    #[inline]
    fn from(i: ImageError) -> Error {
        Self::Image(i)
    }
}

impl From<JoinError> for Error {
    #[inline]
    fn from(j: JoinError) -> Error {
        Self::Join(j)
    }
}

impl From<hound::Error> for Error {
    #[inline]
    fn from(h: hound::Error) -> Error {
        Self::Hound(h)
    }
}

impl From<XmlError> for Error {
    #[inline]
    fn from(x: XmlError) -> Error {
        Self::Xml(x)
    }
}

pub type Result<T = ()> = std::result::Result<T, Error>;
