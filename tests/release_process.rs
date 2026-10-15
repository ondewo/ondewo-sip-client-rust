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

//! The order of `make release` and the credential handling of `make ondewo_release`.
//!
//! Every ONDEWO SIP client implements the same release contract, so the release driver in
//! ondewo-sip-api can treat all of them alike: a read-only `release_preflight` first, the build
//! and the whole suite before anything is committed, a commit that cannot fail silently, and a
//! `release_verify` that only passes once the registry serves the version. crates.io is published
//! by `.github/workflows/release.yml` from the tag, so here push, branch and tag come right before
//! that publish, and the recipe WAITS for it before it says the release is finished.

const MAKEFILE: &str = include_str!("../Makefile");

/// The recipe of `target`: its tab-indented lines plus the column-0 comments between them.
fn recipe(target: &str) -> Vec<&'static str> {
    let header: String = format!("\n{target}:");
    let start: usize = MAKEFILE
        .find(&header)
        .unwrap_or_else(|| panic!("the Makefile defines {target}"));
    MAKEFILE[start + 1..]
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with('\t') || line.starts_with('#'))
        .filter(|line| line.starts_with('\t'))
        .collect()
}

/// The prerequisites of `target`, i.e. what stands between `target:` and `##`.
fn prerequisites(target: &str) -> Vec<&'static str> {
    let line: &str = MAKEFILE
        .lines()
        .find(|line| line.starts_with(&format!("{target}:")))
        .unwrap_or_else(|| panic!("the Makefile defines {target}"));
    line[target.len() + 1..]
        .split("##")
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect()
}

/// The index of the one recipe line that contains `needle`.
fn position(lines: &[&str], needle: &str) -> usize {
    let found: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "`{needle}` must appear exactly once: {lines:#?}"
    );
    found[0]
}

#[test]
fn the_release_runs_its_steps_in_the_contract_order() {
    let release: Vec<&str> = recipe("release");
    let steps: [&str; 12] = [
        "$(MAKE) release_preflight",
        "$(MAKE) build",
        "$(MAKE) release_tests",
        "git add ",
        "git commit",
        "git push\t",
        "$(MAKE) create_release_branch",
        "$(MAKE) create_release_tag",
        "$(MAKE) release_verify_crate",
        "$(MAKE) build_gh_release",
        "$(MAKE) release_verify",
        "Release Finished",
    ];
    // Three steps are matched by how their line ENDS (`$(MAKE) build` is also the start of
    // `$(MAKE) build_gh_release`), the others by a unique substring.
    let positions: Vec<usize> = steps
        .iter()
        .map(|step| match *step {
            "git push\t" => release
                .iter()
                .position(|line| line.trim() == "git push")
                .expect("release pushes the release commit"),
            "$(MAKE) build" => release
                .iter()
                .position(|line| line.ends_with("$(MAKE) build"))
                .expect("release builds"),
            "$(MAKE) release_verify" => release
                .iter()
                .position(|line| line.trim() == "$(MAKE) release_verify")
                .expect("release ends with release_verify"),
            _ => position(&release, step),
        })
        .collect();
    for (pair, window) in positions.windows(2).enumerate() {
        assert!(
            window[0] < window[1],
            "`{}` must come before `{}`",
            steps[pair],
            steps[pair + 1]
        );
    }
    assert_eq!(
        release.last().map(|line| line.trim()),
        Some("@echo \"Release Finished\""),
        "nothing may run after the release reports itself finished"
    );
}

#[test]
fn no_release_step_can_fail_silently() {
    for target in [
        "release",
        "release_preflight",
        "release_preflight_checks",
        "release_tests",
        "release_verify",
        "release_verify_checks",
        "ondewo_release",
    ] {
        for line in recipe(target) {
            let command: &str = line.trim_start_matches('\t').trim_start_matches('@');
            assert!(
                !command.starts_with('-'),
                "{target}: `{line}` ignores its exit status"
            );
            assert!(!command.contains("|| true"), "{target}: `{line}`");
        }
    }
    let release: Vec<&str> = recipe("release");
    let commit: &str = release[position(&release, "git commit")];
    assert!(
        commit.contains("git diff --cached --quiet || git commit --no-verify"),
        "commit only when something is staged, and fail when that commit fails: {commit}"
    );
}

#[test]
fn the_suite_is_the_whole_gate() {
    let tests: String = recipe("release_tests").join("\n");
    for step in [
        "$(MAKE) test",
        "$(MAKE) check_build",
        "$(MAKE) cargo_fmt_check",
        "$(MAKE) cargo_doc",
        "$(MAKE) publish_crate_dry_run",
        "$(MAKE) precommit_hooks_run_all_files",
    ] {
        assert!(tests.contains(step), "release_tests must run {step}");
    }
    assert!(recipe("test").join("\n").contains("--all-targets --locked"));
    assert!(recipe("cargo_build").join("\n").contains("--locked"));
}

#[test]
fn the_preflight_is_read_only_and_checks_everything_first() {
    let preflight: String = recipe("release_preflight_checks").join("\n");
    for check in [
        "check_host_tools",
        "spc",
        "check_release_notes",
        "check_release_credentials",
    ] {
        assert!(preflight.contains(&format!("$(MAKE) {check}")), "{check}");
    }
    let credentials: String = recipe("check_release_credentials").join("\n");
    assert!(credentials.contains("--jq .permissions.push"));
    assert!(credentials.contains("test \"$$push\" = \"true\""));
    assert!(credentials.contains("secret list"));
    assert!(credentials.contains("grep -Fxq CARGO_REGISTRY_TOKEN"));
    assert!(credentials.contains("https://crates.io/api/v1/me"));
    for line in recipe("release_preflight")
        .iter()
        .chain(recipe("release_preflight_checks").iter())
        .chain(recipe("check_release_credentials").iter())
        .chain(recipe("spc").iter())
    {
        for mutating in [
            "git push",
            "git commit",
            "git tag ",
            "release create",
            "cargo publish",
        ] {
            assert!(
                !line.contains(mutating),
                "the preflight must not run `{mutating}`"
            );
        }
    }
}

#[test]
fn spc_matches_exact_refs_and_asks_crates_io() {
    let spc: String = recipe("spc").join("\n");
    assert!(spc.contains(
        "git ls-remote --exit-code --heads origin \"refs/heads/release/${ONDEWO_SIP_VERSION}\""
    ));
    assert!(
        spc.contains("git ls-remote --exit-code --tags origin \"refs/tags/${ONDEWO_SIP_VERSION}\"")
    );
    assert!(spc.contains("git rev-parse -q --verify \"refs/tags/${ONDEWO_SIP_VERSION}\""));
    assert!(spc.contains("${CRATES_IO_VERSION_URL}"));
    assert!(
        !spc.contains("| grep"),
        "an unanchored grep matches 9.1.10 for 9.1.1"
    );
    assert!(
        !spc.contains("& exit"),
        "`& exit 1` backgrounds the exit and passes"
    );
}

#[test]
fn the_release_only_finishes_once_crates_io_serves_the_version() {
    let verify: String = recipe("release_verify_checks").join("\n");
    assert!(verify.contains("git ls-remote --exit-code --tags origin"));
    assert!(verify.contains("release view"));
    assert!(verify.contains("--json body"));
    assert!(verify.contains("$(MAKE) release_verify_crate"));
    let crate_check: String = recipe("release_verify_crate").join("\n");
    assert!(crate_check.contains("--workflow release.yml"));
    assert!(crate_check.contains("run watch"));
    assert!(crate_check.contains("--exit-status"));
    assert!(crate_check.contains("${CRATES_IO_VERSION_URL}"));
    assert!(crate_check.contains("-A \"${CRATES_IO_USER_AGENT}\""));
    assert!(crate_check.contains("${VERIFY_ATTEMPTS}"));
}

#[test]
fn ondewo_release_deletes_the_credentials_clone_before_the_release_starts() {
    assert_eq!(prerequisites("ondewo_release"), vec!["spc"]);
    let lines: Vec<&str> = recipe("ondewo_release");
    let trap: usize = position(&lines, "trap 'rm -rf \"$$owned\"' EXIT INT TERM");
    let clone: usize = position(&lines, "git clone");
    let load: usize = position(&lines, "account_github.env");
    let delete: usize = position(&lines, "then rm -rf \"$$owned\"");
    let release: usize = position(&lines, "$(SUBMAKE) release");
    assert!(
        trap < clone,
        "the trap must be armed before the clone exists"
    );
    assert!(clone < load && load < delete, "clone, load, then delete");
    assert!(
        delete < release,
        "the clone must be gone before `$(SUBMAKE) release`"
    );
    assert!(lines[clone].contains("\"$$accounts\""));
    assert!(
        lines.iter().any(|line| line.contains("owned=`mktemp -d`")),
        "the clone lives outside the repository, where no docker mount reaches it"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("[ -d \"${DEVOPS_ACCOUNT_DIR}\" ]")),
        "a DEVOPS_ACCOUNT_DIR handed in by the release driver is used instead of a new clone"
    );
    // The recipe is ONE shell: every line but the last continues the previous one, so the trap,
    // the variables and the delete all belong to the shell that starts the release.
    for line in &lines[..lines.len() - 1] {
        assert!(line.ends_with('\\'), "not continued: {line}");
    }
}

#[test]
fn nothing_mounts_the_repository_root_as_a_compiler_output() {
    let generate: String = recipe("generate_ondewo_protos").join("\n");
    assert!(generate.contains("-v ${CURDIR}/${PROTO_OUTPUT_DIR}:/output-volume"));
    assert!(!generate.contains("-v ${shell pwd}:/output-volume"));
    let image: &str = MAKEFILE
        .lines()
        .find_map(|line| line.strip_prefix("PROTO_COMPILER_IMAGE="))
        .expect("the Makefile defines PROTO_COMPILER_IMAGE");
    assert!(!image.ends_with(":latest"), "{image}");
    assert!(
        !MAKEFILE.contains("sh build.sh"),
        "build.sh tags the image :latest"
    );
    for pinned in ["RUST_IMAGE=", "FROM "] {
        let text: &str = if pinned == "FROM " {
            include_str!("../Dockerfile.release-tools")
        } else {
            MAKEFILE
        };
        let line: &str = text
            .lines()
            .find(|line| line.starts_with(pinned))
            .unwrap_or_else(|| panic!("no {pinned} line"));
        assert!(line.contains("@sha256:"), "pin the image by digest: {line}");
    }
}

/// The release driver runs `release_preflight` and `release_verify` standalone, with nothing but an
/// absolute `DEVOPS_ACCOUNT_DIR` in the environment, so both load the credentials they lack from
/// that directory first - into the shell that then runs their checks.
#[test]
fn preflight_and_verify_load_missing_credentials_from_the_devops_directory() {
    for target in ["release_preflight", "release_verify"] {
        assert_eq!(
            recipe(target),
            vec![format!(
                "\t@$(LOAD_DEVOPS_CREDENTIALS); $(SUBMAKE) --no-print-directory {target}_checks"
            )],
            "{target} must load the credentials, then run its checks in the same shell"
        );
        assert!(
            prerequisites(target).is_empty(),
            "{target} must not run a check before the credentials are loaded"
        );
    }
    let start: usize = MAKEFILE
        .find("\nLOAD_DEVOPS_CREDENTIALS=")
        .expect("the Makefile defines LOAD_DEVOPS_CREDENTIALS");
    let load: &str = &MAKEFILE[start + 1..];
    let load: &str = &load[..load
        .find("\n\tfi\n")
        .expect("LOAD_DEVOPS_CREDENTIALS ends in fi")];
    // Only from an existing directory, each value only when the environment lacks it, anchored,
    // through `set -a` + eval - never on a command line, never echoed.
    assert!(load.contains("[ -d \"${DEVOPS_ACCOUNT_DIR}\" ]"));
    assert!(load.contains("set -a"));
    for (secret, file) in [
        ("GITHUB_GH_TOKEN", "account_github.env"),
        ("CARGO_REGISTRY_TOKEN", "account_cargo.env"),
    ] {
        assert!(
            load.contains(&format!(
                "[ -n \"$${{{secret}:-}}\" ] || eval \"`grep -h -E '^{secret}=' \"${{DEVOPS_ACCOUNT_DIR}}/{file}\""
            )),
            "{secret} from {file}"
        );
    }
    assert!(!load.contains("echo"), "nothing in the loader may print");
}

/// GNU make executes every recipe line that references `$(MAKE)` even under `make -n`, so a recipe
/// that clones devops-accounts or loads credentials must start its sub-make through the `SUBMAKE`
/// alias - otherwise `make -n ondewo_release` clones and loads just to print the plan.
#[test]
fn a_dry_run_never_clones_or_loads_credentials() {
    assert!(MAKEFILE.contains("\nSUBMAKE=$(MAKE)\n"));
    for target in [
        "ondewo_release",
        "ondewo_publish_crate",
        "release_preflight",
        "release_verify",
    ] {
        for line in recipe(target) {
            assert!(
                !line.contains("$(MAKE)"),
                "{target}: `{line}` runs even under `make -n`"
            );
        }
    }
}
