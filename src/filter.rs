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

use once_cell::sync::Lazy;
use regex::Regex;

// Words we don't want in our videos.
const FILTER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new("(?i)(nigg|fag|porn|masturb|biden|trump)").expect("Regex failed to compile")
});

#[inline]
pub fn filter_text(s: &str) -> crate::Result {
    if FILTER_REGEX.is_match(s) {
        Err(crate::Error::DisallowedWord)
    } else {
        Ok(())
    }
}

#[inline]
pub fn filter_pass(s: String) -> crate::Result<String> {
    filter_text(&s).map(|()| s)
}

#[test]
fn test_filter() {
    filter_text("I am some normal text").unwrap();
    filter_text("I masturbate to Joe Biden").unwrap_err();
    filter_text("PoRn").unwrap_err();
}
