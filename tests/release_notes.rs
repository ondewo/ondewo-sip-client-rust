// Copyright 2021-2026 ONDEWO GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The GitHub release body is the RELEASE.md section that `make release_notes` prints.
//!
//! The slice starts at the heading `## Release ONDEWO SIP Rust Client <version>` - matched
//! EXACTLY, so `9.1.1` never selects the section of `9.1.10` - and ends at the next `*****` line.
//! A heading spelled any other way gives an empty slice, which `check_release_notes` refuses
//! before anything is pushed; a range that ended at the first `**` instead would cut the notes at
//! their first bold span. These tests run the Makefile's own slice through make and perl rather
//! than a re-implementation of it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const RELEASE_NOTES: &str = include_str!("../RELEASE.md");
const MAKEFILE: &str = include_str!("../Makefile");
const HEADING_PREFIX: &str = "## Release ONDEWO SIP Rust Client ";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Run `make -s <target> ONDEWO_SIP_VERSION=<version> RELEASE_NOTES_FILE=<file>`.
fn make(target: &str, version: &str, notes_file: &Path) -> Output {
    Command::new("make")
        .current_dir(root())
        // A `make release` that runs this suite must not hand its own flags (-n, a jobserver)
        // to the make started here.
        .env_remove("MAKEFLAGS")
        .env_remove("MAKELEVEL")
        .env_remove("MFLAGS")
        .args(["-s", "--no-print-directory", target])
        .arg(format!("ONDEWO_SIP_VERSION={version}"))
        .arg(format!("RELEASE_NOTES_FILE={}", notes_file.display()))
        .output()
        .expect("make is on PATH")
}

fn release_notes_of(version: &str, notes_file: &Path) -> String {
    let output: Output = make("release_notes", version, notes_file);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("the notes are UTF-8")
}

fn headings() -> Vec<&'static str> {
    RELEASE_NOTES
        .lines()
        .filter(|line| line.starts_with("## "))
        .collect()
}

fn version_of(heading: &str) -> &str {
    heading.strip_prefix(HEADING_PREFIX).unwrap()
}

fn fixture(name: &str, text: &str) -> PathBuf {
    let directory: PathBuf = root().join("target").join("release-notes-fixtures");
    std::fs::create_dir_all(&directory).expect("target/ is writable");
    let path: PathBuf = directory.join(name);
    std::fs::write(&path, text).expect("the fixture is writable");
    path
}

#[test]
fn the_slice_is_anchored_and_stops_at_its_separator() {
    let slice: &str = MAKEFILE
        .lines()
        .find_map(|line| line.strip_prefix("RELEASE_NOTES_SLICE="))
        .expect("the Makefile defines RELEASE_NOTES_SLICE");
    assert!(slice.contains(r"/^\x23+ ${RELEASE_NOTES_HEADING} \Q${ONDEWO_SIP_VERSION}\E$$/"));
    assert!(slice.contains(r"exit if /^\*{5}/"));
    assert!(MAKEFILE.contains("--notes-file -"));
}

#[test]
fn a_version_never_selects_the_section_of_a_longer_one() {
    let notes: PathBuf = fixture(
        "prefix.md",
        "# Release History\n\n*****************\n\n\
         ## Release ONDEWO SIP Rust Client 9.1.10\n\n* ten\n\n*****************\n\n\
         ## Release ONDEWO SIP Rust Client 9.1.1\n\n* one, see **9.1.10** too\n\n\
         *****************\n\n\
         ## Release ONDEWO SIP Rust Client 9.1.1\n\n* a later mention must not restart the slice\n\n\
         *****************\n",
    );
    let one: String = release_notes_of("9.1.1", &notes);
    assert!(
        one.starts_with("## Release ONDEWO SIP Rust Client 9.1.1\n"),
        "{one}"
    );
    assert!(one.contains("* one, see **9.1.10** too"), "{one}");
    assert!(!one.contains("* ten"), "{one}");
    assert!(!one.contains("later mention"), "{one}");
    let ten: String = release_notes_of("9.1.10", &notes);
    assert!(ten.contains("* ten") && !ten.contains("* one"), "{ten}");
    // A dot is a dot: 9x1x1 is not 9.1.1.
    assert_eq!(release_notes_of("9x1x1", &notes), "");
    assert_eq!(release_notes_of("9.1", &notes), "");
    assert!(!make("check_release_notes", "9.1", &notes).status.success());
    assert!(make("check_release_notes", "9.1.1", &notes)
        .status
        .success());
}

#[test]
fn a_heading_without_notes_is_refused() {
    let notes: PathBuf = fixture(
        "empty.md",
        "## Release ONDEWO SIP Rust Client 1.2.3\n\n\n*****************\n",
    );
    assert_eq!(
        release_notes_of("1.2.3", &notes).lines().next(),
        Some("## Release ONDEWO SIP Rust Client 1.2.3")
    );
    assert!(!make("check_release_notes", "1.2.3", &notes)
        .status
        .success());
}

#[test]
fn every_release_heading_uses_the_spelling_the_makefile_slices() {
    let headings = headings();
    assert!(!headings.is_empty());
    for heading in &headings {
        let version = heading.strip_prefix(HEADING_PREFIX);
        assert!(
            version.is_some_and(|version| {
                let parts: Vec<&str> = version.split('.').collect();
                parts.len() == 3
                    && parts
                        .iter()
                        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            }),
            "unexpected heading spelling: {heading:?}"
        );
    }
    let mut versions: Vec<&str> = headings.iter().map(|heading| version_of(heading)).collect();
    versions.sort_unstable();
    versions.dedup();
    assert_eq!(versions.len(), headings.len(), "a version has two sections");
}

#[test]
fn every_section_ends_at_its_separator_and_holds_only_its_own_notes() {
    let release_md: PathBuf = root().join("RELEASE.md");
    for heading in headings() {
        let slice: String = release_notes_of(version_of(heading), &release_md);
        let lines: Vec<&str> = slice.lines().collect();
        assert_eq!(lines[0], heading);
        assert!(lines[lines.len() - 1].starts_with("*****"), "{heading}");
        assert!(
            lines[1..].iter().all(|line| !line.starts_with("## ")),
            "the section of {heading:?} runs into the next one"
        );
        assert!(
            make("check_release_notes", version_of(heading), &release_md)
                .status
                .success(),
            "the section of {heading:?} is empty"
        );
    }
}

#[test]
fn the_current_version_has_non_empty_release_notes() {
    let version = MAKEFILE
        .lines()
        .find_map(|line| line.strip_prefix("ONDEWO_SIP_VERSION="))
        .expect("the Makefile sets ONDEWO_SIP_VERSION")
        .trim();
    let output: Output = make("check_release_notes", version, &root().join("RELEASE.md"));
    assert!(output.status.success(), "{version}: {output:?}");
}
