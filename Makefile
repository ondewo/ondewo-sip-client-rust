# =====================================================================================
# ondewo-sip-client-rust - Makefile
#
# The ONDEWO SIP (SIP Telephony) gRPC client for rust. The crate is generated
# from the protos of the ondewo-sip-api submodule by the ondewo-rust-proto-compiler image that
# the ondewo-proto-compiler submodule builds - only src/api is generated, everything else in
# this repository is hand-written.
#
# Quick start:
#   make help                 # list every documented target
#   make makefile_chapters    # list the section headers below
#   make build                # submodules -> compiler image -> stubs -> cargo build
#   make test                 # cargo test
# =====================================================================================

# ---------------- BEFORE RELEASE ----------------
# 1 - Update the version number (ONDEWO_SIP_VERSION below)
# 2 - Update RELEASE.md
# 3 - make build
# -------------- Release Process Steps (make ondewo_release) --------------
# 1 - spc: the version is neither a release branch / tag (origin and local) nor on crates.io
# 2 - load the credentials from devops-accounts, then DELETE that clone before anything runs
# 3 - release_preflight: host tools, spc, RELEASE.md section, credentials valid (read-only)
# 4 - build + the whole test suite, without any credential in their environment
# 5 - release commit, push, release branch, release tag
# 6 - crates.io: .github/workflows/release.yml publishes from the tag; release_verify_crate waits
#     for that run and for crates.io to serve the version
# 7 - GitHub release, then release_verify (tag, non-empty release body, crate served)

########################################################
# 		Variables
########################################################

# MUST BE THE SAME AS THE SIP API IN MAJOR AND MINOR VERSION NUMBER
# example: API 2.9.0 --> Client 2.9.X
ONDEWO_SIP_VERSION=5.7.2

# Submodule pins - `make checkout_defined_submodule_versions` checks out exactly these.
# Pin the API to `tags/<api version>` before cutting a release; a branch is for development only.
ONDEWO_SIP_API_GIT_BRANCH=tags/5.7.0
# The compiler has to be 5.15.4 or newer: 5.15.4 pre-warms tonic tls-native-roots, which
# src/channel.rs needs, and an image older than 5.15.2 builds its crate without the README.md
# that Cargo.toml's `readme` names (generate_ondewo_protos stages it).
ONDEWO_PROTO_COMPILER_GIT_BRANCH=tags/5.17.0

# Submodule directories - these MUST match the paths in .gitmodules
ONDEWO_SIP_API_DIR=ondewo-sip-api
ONDEWO_PROTO_COMPILER_DIR=ondewo-proto-compiler

# The generation contract with the proto compiler. The image TAG is the only contract between
# this repository and the compiler - `make build_compiler` rebuilds it from the submodule.
# The image ENTRYPOINT takes two positional arguments, BOTH relative to /input-volume:
#   <relative_protos_dir>   the protoc -I root every `import "x/y.proto";` resolves against
#   <target_subdir>         the sub-directory whose protos are the compilation entry points;
#                           google/ is pulled in only as a resolved dependency
# The tag is the compiler VERSION, never `latest`: the sip/csi/sip clients pin different compiler
# releases, and one shared `latest` tag lets one product's release overwrite another's compiler.
# The submodule's build.sh hard-codes `:latest`, so build_compiler runs docker build itself.
PROTO_COMPILER_VERSION=$(subst /,_,$(patsubst tags/%,%,${ONDEWO_PROTO_COMPILER_GIT_BRANCH}))
PROTO_COMPILER_IMAGE=ondewo-rust-proto-compiler:${PROTO_COMPILER_VERSION}
ONDEWO_PROTOS_DIR=${ONDEWO_SIP_API_DIR}
ONDEWO_PROTOS_TARGET_DIR=ondewo

# Staging directories handed to the image as /input-volume and /output-volume (see
# generate_ondewo_protos). Neither is the repository root, so nothing else in the working tree is
# ever mounted into the compiler container.
PROTO_INPUT_DIR=.proto-input
PROTO_OUTPUT_DIR=.proto-output

# Host tools. A release needs only docker, make, git, perl and curl on the host; cargo, rustfmt,
# pre-commit and gh are used from the host when present and otherwise run in pinned images:
#   RUST_IMAGE                - the official rust image, by version AND digest
#   Dockerfile.release-tools  - gh (pinned version, sha256-checked) + pre-commit (pinned), built
#                               from stdin (no build context) and tagged by the Dockerfile's hash
RUST_IMAGE=rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e
RELEASE_TOOLS_IMAGE=ondewo-sip-client-rust-release-tools:$(shell sha256sum Dockerfile.release-tools 2>/dev/null | cut -c1-16)
DOCKER_AS_ME=docker run --rm --user $(shell id -u):$(shell id -g) -e HOME=/tmp -v $(CURDIR):/work -w /work
CARGO_IN_DOCKER=${DOCKER_AS_ME} -e CARGO_HOME=/work/target/docker-cargo-home ${RUST_IMAGE} cargo
RUSTFMT_IN_DOCKER=${DOCKER_AS_ME} ${RUST_IMAGE} rustfmt
PRE_COMMIT_IN_DOCKER=${DOCKER_AS_ME} -e PRE_COMMIT_HOME=/work/target/docker-pre-commit-home ${RELEASE_TOOLS_IMAGE} pre-commit
# -i so `--notes-file -` can read the notes from stdin; -e GH_TOKEN forwards the variable by NAME,
# so the token never reaches docker's argv.
GH_IN_DOCKER=docker run --rm -i -e GH_TOKEN ${RELEASE_TOOLS_IMAGE} gh
CARGO?=$(if $(shell command -v cargo 2>/dev/null),cargo,${CARGO_IN_DOCKER})
RUSTFMT?=$(if $(shell command -v rustfmt 2>/dev/null),rustfmt,${RUSTFMT_IN_DOCKER})
PRE_COMMIT?=$(if $(shell command -v pre-commit 2>/dev/null),pre-commit,${PRE_COMMIT_IN_DOCKER})
GH?=$(if $(shell command -v gh 2>/dev/null),gh,${GH_IN_DOCKER})
# publish_crate only: the one cargo invocation that may see the crates.io token (-e by NAME).
CARGO_PUBLISH?=$(if $(shell command -v cargo 2>/dev/null),cargo,${DOCKER_AS_ME} -e CARGO_HOME=/work/target/docker-cargo-home -e CARGO_REGISTRY_TOKEN ${RUST_IMAGE} cargo)

# The recipes that clone or load credentials start their sub-make through this alias rather than
# through a literal $$(MAKE): GNU make EXECUTES a recipe line that references $$(MAKE) even under
# `make -n`, so `make -n ondewo_release` would clone devops-accounts and load the credentials just
# to show the plan. Referenced through another variable, the line is only printed.
SUBMAKE=$(MAKE)

# Build and test steps of a release run under this prefix, so the release credentials never reach
# cargo, its build scripts, the test binaries, docker or the pre-commit hooks - only the gh / curl
# steps that need a token see one.
WITHOUT_CREDENTIALS=env -u GITHUB_GH_TOKEN -u CARGO_REGISTRY_TOKEN -u GH_TOKEN -u GITHUB_TOKEN

# Coverage gate - KEEP IN SYNC with .github/workflows/ci.yml, which runs the same two values.
# Excluded from the metric: the generated stubs (machine output, not authored logic) and the
# tests/examples themselves (the measuring instrument).
COVERAGE_EXCLUDE_REGEX=(^|/)(src/api/|tests/|examples/)
COVERAGE_MIN_LINES=100

# You need to setup an access token at https://github.com/settings/tokens - permissions are important
GITHUB_GH_TOKEN?=ENTER_YOUR_TOKEN_HERE
# You need to setup an API token at https://crates.io/settings/tokens, scoped to publish-new +
# publish-update. `cargo publish` reads it from the environment, so it never appears on a command
# line and never reaches the build log.
CARGO_REGISTRY_TOKEN?=ENTER_HERE_YOUR_CARGO_REGISTRY_TOKEN
# This Makefile exports NOTHING by itself: a credential reaches a recipe only when it is in the
# environment make was started with (ondewo_release puts it there), and recipes read it as $$NAME.

# crates.io refuses an upload whose .crate tarball is larger than this (10 MiB, the default limit).
# The check is server-side, so `cargo publish --dry-run` cannot catch it - publish_crate_dry_run does.
CRATE_MAX_BYTES=10485760

# The release notes of ONDEWO_SIP_VERSION: from the heading that matches EXACTLY (anchored, the
# version dots taken literally by \Q..\E, so 9.1.1 never matches 9.1.10) down to the next *****
# separator, then stop - a later mention of the heading cannot restart the slice. Ending on
# /\*\*/ instead would cut the notes at their first markdown **bold** span. \x23 is `#`, which make
# would otherwise read as a comment.
RELEASE_NOTES_FILE=RELEASE.md
RELEASE_NOTES_HEADING=Release ONDEWO SIP Rust Client
RELEASE_NOTES_SLICE=perl -ne '$$in = 1 if /^\x23+ ${RELEASE_NOTES_HEADING} \Q${ONDEWO_SIP_VERSION}\E$$/; if ($$in) { print; exit if /^\*{5}/ }' ${RELEASE_NOTES_FILE}
CURRENT_RELEASE_NOTES=`${RELEASE_NOTES_SLICE}`

CRATE_NAME=ondewo-sip-client
GH_REPO_SLUG=ondewo/ondewo-sip-client-rust
GH_REPO="https://github.com/${GH_REPO_SLUG}"
# crates.io's crawler policy requires a User-Agent that identifies the caller.
CRATES_IO_USER_AGENT=ondewo-sip-client-rust-release (office@ondewo.com)
CRATES_IO_VERSION_URL=https://crates.io/api/v1/crates/${CRATE_NAME}/${ONDEWO_SIP_VERSION}
# release_verify polls the registry this often before it gives up (20 x 30 s = 10 minutes).
VERIFY_ATTEMPTS=20
VERIFY_INTERVAL_S=30

DEVOPS_ACCOUNT_GIT=ondewo-devops-accounts
DEVOPS_ACCOUNT_REPO=git@bitbucket.org:ondewo/${DEVOPS_ACCOUNT_GIT}.git
# Set by the ondewo-sip-api release driver to its single temporary clone: when it names an
# existing directory, ondewo_release reads the credentials from there and neither clones nor
# deletes it (the driver owns it). Unset, ondewo_release clones into a fresh `mktemp -d` OUTSIDE
# this repository and deletes that clone before the release starts.
DEVOPS_ACCOUNT_DIR?=
# The release driver also runs release_preflight and release_verify STANDALONE, with nothing but an
# absolute DEVOPS_ACCOUNT_DIR set. Those two therefore load what they need from that directory -
# each credential only when the environment does not already carry it (inside `make release` it
# does) - into their own recipe shell: anchored grep, `set -a` + eval, so no value reaches an argv or
# the log. A no-op when DEVOPS_ACCOUNT_DIR is unset or not a directory.
LOAD_DEVOPS_CREDENTIALS=if [ -n "${DEVOPS_ACCOUNT_DIR}" ] && [ -d "${DEVOPS_ACCOUNT_DIR}" ]; then \
		set -a; \
		[ -n "$${GITHUB_GH_TOKEN:-}" ] || eval "`grep -h -E '^GITHUB_GH_TOKEN=' "${DEVOPS_ACCOUNT_DIR}/account_github.env" 2>/dev/null`"; \
		[ -n "$${CARGO_REGISTRY_TOKEN:-}" ] || eval "`grep -h -E '^CARGO_REGISTRY_TOKEN=' "${DEVOPS_ACCOUNT_DIR}/account_cargo.env" 2>/dev/null`"; \
		set +a; \
	fi

# Define colors globally (reused for [INFO]/[SUCCESS]/[WARN]/[ERROR] log lines in recipes)
BLUE   := \033[1;34m
GREEN  := \033[0;32m
YELLOW := \033[1;33m
RED    := \033[0;31m
NC     := \033[0m

.DEFAULT_GOAL := help

########################################################
#       ONDEWO Standard Make Targets
########################################################

setup_developer_environment_locally: install_rust_toolchain install_precommit_hooks ## Ready a fresh laptop: rust toolchain + pre-commit hooks

install_rust_toolchain: ## Install rustup/cargo if missing and add the rustfmt and clippy components
	@command -v cargo >/dev/null 2>&1 || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
# rustup's installer puts cargo on PATH for NEW shells only, so source its env for this one -
# otherwise the very first run of this target installs rustup and then cannot call it.
	[ -f "$$HOME/.cargo/env" ] && . "$$HOME/.cargo/env"; rustup component add rustfmt clippy

install_precommit_hooks: ## Installs pre-commit hooks and sets them up for the ondewo-sip-client-rust repo
	@command -v pre-commit >/dev/null 2>&1 || uv tool install pre-commit || pipx install pre-commit || pip install --user pre-commit
	pre-commit install
	pre-commit install --hook-type commit-msg

precommit_hooks_run_all_files: release_tools_image ## Runs all pre-commit hooks on all files and not just the changed ones
	${PRE_COMMIT} run --all-files

help: ## Print usage info about help targets
	# (first comment after target starting with double hashes ##)
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' Makefile | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-40s\033[0m %s\n", $$1, $$2}'

makefile_chapters: ## Shows all sections of Makefile
	@echo `cat Makefile| grep "########################################################" -A 1 | grep -v "########################################################"`

TEST: ## Prints some important variables
	@echo "Release Notes: \n \n$(CURRENT_RELEASE_NOTES)"
	@echo "GH Token: \t $(if $(filter-out ENTER_YOUR_TOKEN_HERE,$(GITHUB_GH_TOKEN)),<set>,<unset>)"
	@echo "Cargo Token: \t $(if $(filter-out ENTER_HERE_YOUR_CARGO_REGISTRY_TOKEN,$(CARGO_REGISTRY_TOKEN)),<set>,<unset>)"
	@echo "Compiler Image:  $(PROTO_COMPILER_IMAGE)"
	@echo "Protos: \t $(ONDEWO_PROTOS_DIR)/$(ONDEWO_PROTOS_TARGET_DIR)"

check_build: ## Checks if all built proto-code is there
# prost writes ONE file per proto PACKAGE (flat_output_dir), named "<package>.rs" - so the
# check is per package, not per .proto file, and src/api/mod.rs is the barrel that includes them.
	@rm -f build_check.txt
	@find ${ONDEWO_PROTOS_DIR}/${ONDEWO_PROTOS_TARGET_DIR} -type f -name "*.proto" -exec sed -n 's|^[[:space:]]*package[[:space:]][[:space:]]*\([A-Za-z0-9_.]*\)[[:space:]]*;.*|\1|p' {} \; > build_check.txt
	@sort -u build_check.txt -o build_check.txt
	@test -s build_check.txt || { echo "$(RED)[ERROR]$(NC) No proto packages found under ${ONDEWO_PROTOS_DIR}/${ONDEWO_PROTOS_TARGET_DIR}"; rm -f build_check.txt; exit 1; }
	@test -f src/api/mod.rs || { echo "$(RED)[ERROR]$(NC) src/api/mod.rs is missing - run 'make generate_ondewo_protos'"; rm -f build_check.txt; exit 1; }
	@while read -r package; do \
		test -f "src/api/$$package.rs" || { echo "$(RED)[ERROR]$(NC) No rust stub for proto package $$package (expected src/api/$$package.rs)"; rm -f build_check.txt; exit 1; }; \
	done < build_check.txt
	@rm -f build_check.txt
	@echo "$(GREEN)[SUCCESS]$(NC) A generated stub exists for every proto package"

########################################################
#       Repo Specific Make Targets
########################################################
#		Build

build: update_submodules checkout_defined_submodule_versions build_compiler update_cargo_version generate_ondewo_protos check_build cargo_build ## Build source code: submodules -> compiler image -> stubs -> cargo build
	@echo "$(GREEN)[SUCCESS]$(NC) Build of ondewo-sip-client-rust ${ONDEWO_SIP_VERSION} finished"

build_compiler: ## Build proto compiler docker image from the ondewo-proto-compiler submodule
	@echo "$(BLUE)[INFO]$(NC) Building ${PROTO_COMPILER_IMAGE} from ${ONDEWO_PROTO_COMPILER_DIR}/rust ..."
# What the submodule's build.sh runs, but tagged with the compiler version instead of `latest`.
	docker build --no-cache -t ${PROTO_COMPILER_IMAGE} ${ONDEWO_PROTO_COMPILER_DIR}/rust
	@echo "$(GREEN)[SUCCESS]$(NC) Built ${PROTO_COMPILER_IMAGE}"

generate_ondewo_protos: ## Generate rust code from proto files into src/api
	@echo "$(BLUE)[INFO]$(NC) Generating rust stubs from ${ONDEWO_PROTOS_DIR}/${ONDEWO_PROTOS_TARGET_DIR} ..."
	@test -d ${ONDEWO_PROTOS_DIR}/${ONDEWO_PROTOS_TARGET_DIR} || { echo "$(RED)[ERROR]$(NC) '${ONDEWO_PROTOS_DIR}/${ONDEWO_PROTOS_TARGET_DIR}' is missing - run 'make update_submodules' first"; exit 1; }
# The image COPIES the whole mounted input volume into itself before it compiles, so the repo
# root is deliberately not the input volume: it carries target/ (gigabytes after a release
# build) and .git. Stage exactly what the image consumes instead - the protos, the crate
# manifest, the README.md its `readme` names (the image's `cargo package` refuses the crate
# without it) and the hand-written sources. src/api is left out on purpose: it is entirely
# generated, and the image wipes it in the output volume before copying the new stubs back.
	rm -rf ${PROTO_INPUT_DIR}
	mkdir -p ${PROTO_INPUT_DIR}/src
	cp Cargo.toml README.md ${PROTO_INPUT_DIR}/
	find src -mindepth 1 -maxdepth 1 ! -name api -exec cp -R {} ${PROTO_INPUT_DIR}/src/ \;
	cp -R ${ONDEWO_PROTOS_DIR} ${PROTO_INPUT_DIR}/${ONDEWO_PROTOS_DIR}
# Same image tag and the same positional arguments as the compiler's own
# ondewo-proto-compiler/rust/example/run-compile.sh. Two deliberate differences:
#  * NO -it - a TTY-enabled container breaks every non-interactive caller with
#    "cannot attach stdin to a TTY-enabled container because stdin is not a terminal".
#  * --user, as in ondewo-proto-compiler/rust/Makefile, so the generated src/api,
#    Cargo.toml, Cargo.lock and crate-dist/ are owned by the caller and not by root.
#    The image chmods its working directories a+w so this works.
# The output volume is a staging directory, NOT the repository root: the root can hold anything
# (a credentials clone, a stale target/), and none of it is the container's business. The image
# writes src/, Cargo.toml, Cargo.lock and crate-dist/ there; they are copied back afterwards.
	rm -rf ${PROTO_OUTPUT_DIR}
	mkdir -p ${PROTO_OUTPUT_DIR}
	docker run --rm \
		--user ${shell id -u}:${shell id -g} \
		-v ${CURDIR}/${PROTO_INPUT_DIR}:/input-volume \
		-v ${CURDIR}/${PROTO_OUTPUT_DIR}:/output-volume \
		${PROTO_COMPILER_IMAGE} ${ONDEWO_PROTOS_DIR} ${ONDEWO_PROTOS_TARGET_DIR}
	test -f ${PROTO_OUTPUT_DIR}/src/api/mod.rs
	rm -rf src/api crate-dist
	cp -R ${PROTO_OUTPUT_DIR}/. ./
	rm -rf ${PROTO_INPUT_DIR} ${PROTO_OUTPUT_DIR}
	@echo "$(GREEN)[SUCCESS]$(NC) Generated the rust stubs in src/api"

update_cargo_version: ## Update Version in Cargo.toml
	@perl -0pi -e 's/(\[package\][^\[]*?\nversion = ")[^"]*(")/$${1}${ONDEWO_SIP_VERSION}$${2}/s' Cargo.toml
	@echo "$(GREEN)[SUCCESS]$(NC) Cargo.toml version set to ${ONDEWO_SIP_VERSION}"

# --locked everywhere, as in CI: a Cargo.lock that does not match Cargo.toml is an error, never a
# silent rewrite that leaves the release tree dirty.
cargo_build: ## Compile the crate in release mode
	${CARGO} build --release --locked

test: ## Run the test suite (--all-targets also compiles examples/)
	${CARGO} test --all-targets --locked

release_tests: ## The whole gate a release must pass before anything is committed or pushed
	$(MAKE) test
	$(MAKE) check_build
	$(MAKE) cargo_fmt_check
	$(MAKE) cargo_doc
	$(MAKE) publish_crate_dry_run
	$(MAKE) precommit_hooks_run_all_files

coverage: ## Line coverage of the HAND-WRITTEN sources, gated at COVERAGE_MIN_LINES (as in CI)
# src/api is machine output and tests/ + examples/ are the measuring instrument, so neither
# belongs in the metric - what remains is exactly the hand-written library surface. The generated
# stubs are still exercised, by the behavioural tests under tests/.
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { echo "$(RED)[ERROR]$(NC) cargo-llvm-cov is missing - install it with 'cargo install cargo-llvm-cov --locked'"; exit 1; }
	cargo llvm-cov --locked --summary-only \
		--ignore-filename-regex '${COVERAGE_EXCLUDE_REGEX}' \
		--fail-under-lines ${COVERAGE_MIN_LINES}

cargo_fmt: ## Format the hand-written sources (src/api is generated and never hand-formatted)
	${RUSTFMT} --edition 2021 `find src tests examples -name "*.rs" -not -path "src/api/*"`

cargo_fmt_check: ## Check the formatting of the hand-written sources without changing them (as CI does)
	${RUSTFMT} --check --edition 2021 `find src tests examples -name "*.rs" -not -path "src/api/*"`

cargo_doc: ## Build the crate documentation
	${CARGO} doc --no-deps --locked

clean_generated_api: ## Clear the generated stubs in src/api
	rm -rf src/api

clean: ## Remove cargo build output, the packaged crate and the generation staging directory
	${CARGO} clean
	rm -rf crate-dist ${PROTO_INPUT_DIR} ${PROTO_OUTPUT_DIR} build_check.txt

########################################################
#		Submodules

update_submodules: ## Initialize and update all submodules
	@echo "$(BLUE)[INFO]$(NC) START initializing submodules ..."
	git submodule update --init --recursive
	@echo "$(GREEN)[SUCCESS]$(NC) DONE initializing submodules"

checkout_defined_submodule_versions: ## Update submodule versions to the pins at the top of this Makefile
	@echo "$(BLUE)[INFO]$(NC) START checking out submodules ..."
	git -C ${ONDEWO_SIP_API_DIR} fetch --all
	git -C ${ONDEWO_SIP_API_DIR} checkout ${ONDEWO_SIP_API_GIT_BRANCH}
	git -C ${ONDEWO_PROTO_COMPILER_DIR} fetch --all
	git -C ${ONDEWO_PROTO_COMPILER_DIR} checkout ${ONDEWO_PROTO_COMPILER_GIT_BRANCH}
	@echo "$(GREEN)[SUCCESS]$(NC) DONE checking out submodules"

########################################################
#		Release

release: ## Automate the entire release process (run it through `make ondewo_release`)
	@echo "Start Release"
# Everything that can be refuted without touching origin is refuted FIRST, read-only: host tools,
# version absent everywhere, RELEASE.md section, every credential valid. A failure here leaves no
# commit, no branch, no tag and nothing on crates.io.
	$(MAKE) release_preflight
# Build and the WHOLE suite run before anything is committed or pushed - a red suite must never
# reach a tag, and the tag push is what publishes to crates.io. Neither sees a credential.
	${WITHOUT_CREDENTIALS} $(MAKE) build
	${WITHOUT_CREDENTIALS} $(MAKE) release_tests
	git add Cargo.toml Cargo.lock src Makefile README.md RELEASE.md tests ${ONDEWO_PROTO_COMPILER_DIR} ${ONDEWO_SIP_API_DIR}
	git status --short
# Commit only when something is staged (a re-run after a commit has nothing new); a commit that
# FAILS with something staged stops the release.
	git diff --cached --quiet || git commit --no-verify -m "PREPARING FOR RELEASE ${ONDEWO_SIP_VERSION}"
# crates.io is published by .github/workflows/release.yml FROM THE TAG, so push, release branch and
# tag come right before the publish step; a second publisher here would race that workflow and
# the loser would die on "crate version already uploaded".
	git push
	$(MAKE) create_release_branch
	$(MAKE) create_release_tag
# Wait for the release workflow of the tag to succeed and for crates.io to SERVE the version
# before anything announces the release.
	$(MAKE) release_verify_crate
	$(MAKE) build_gh_release
	$(MAKE) release_verify
	@echo "Release Finished"

release_preflight: ## Read-only: fail before anything is built, committed or pushed (tools, version, notes, credentials)
	@$(LOAD_DEVOPS_CREDENTIALS); $(SUBMAKE) --no-print-directory release_preflight_checks

release_preflight_checks: ## The checks of release_preflight, with the credentials already in the environment
	$(MAKE) check_host_tools
	$(MAKE) spc
	$(MAKE) check_release_notes
	$(MAKE) check_release_credentials
	@echo "$(GREEN)[SUCCESS]$(NC) Preflight for ${ONDEWO_SIP_VERSION} passed - nothing has been changed"

check_host_tools: ## The host tools a release needs; cargo, rustfmt, pre-commit and gh fall back to pinned images
	@for tool in docker make git perl curl; do \
		command -v $$tool >/dev/null 2>&1 || { echo "$(RED)[ERROR]$(NC) '$$tool' is not on PATH - a release needs docker, make, git, perl and curl on the host"; exit 1; }; \
	done
# The tool fallbacks mount this repository into containers, so a credentials clone left here by an
# older release (it used to be cloned into the repository) must not be sitting in it.
	@test ! -e ${DEVOPS_ACCOUNT_GIT} || { echo "$(RED)[ERROR]$(NC) ./${DEVOPS_ACCOUNT_GIT} exists - a stale credentials clone inside the repository; delete it"; exit 1; }
	@docker version >/dev/null 2>&1 || { echo "$(RED)[ERROR]$(NC) the docker daemon is not reachable - the proto compiler (and every tool fallback) runs in docker"; exit 1; }
	@for tool in cargo rustfmt pre-commit gh; do \
		if command -v $$tool >/dev/null 2>&1; then echo "$(BLUE)[INFO]$(NC) $$tool: host (`command -v $$tool`)"; \
		else echo "$(BLUE)[INFO]$(NC) $$tool: not on the host - runs in a pinned docker image"; fi; \
	done
	$(MAKE) release_tools_image
	@echo "$(GREEN)[SUCCESS]$(NC) Every tool the release needs is available"

release_tools_image: ## Build the pinned gh + pre-commit image, only when one of them is missing on the host
	@if command -v gh >/dev/null 2>&1 && command -v pre-commit >/dev/null 2>&1; then exit 0; fi; \
	if docker image inspect ${RELEASE_TOOLS_IMAGE} >/dev/null 2>&1; then exit 0; fi; \
	echo "$(BLUE)[INFO]$(NC) Building ${RELEASE_TOOLS_IMAGE} from Dockerfile.release-tools ..."; \
	docker build -t ${RELEASE_TOOLS_IMAGE} - < Dockerfile.release-tools

check_release_credentials: ## Read-only: every credential the release uses is present AND valid
# Read through the SHELL ($$VAR), never through make ($(VAR)), so a token is never interpolated into
# the recipe text and cannot reach the log; gh gets it as GH_TOKEN for that one call only, so
# nothing is ever stored in ~/.config/gh.
#
# 1. GitHub: the token must be able to PUSH here (branch, tag, release).
	@test -n "$$GITHUB_GH_TOKEN" -a "$$GITHUB_GH_TOKEN" != "ENTER_YOUR_TOKEN_HERE" || { echo "$(RED)[ERROR]$(NC) GITHUB_GH_TOKEN is not set - create one at https://github.com/settings/tokens, or run 'make ondewo_release', which reads it from ${DEVOPS_ACCOUNT_GIT}"; exit 1; }
	@push=`GH_TOKEN="$$GITHUB_GH_TOKEN" ${GH} api repos/${GH_REPO_SLUG} --jq .permissions.push 2>/dev/null`; \
	test "$$push" = "true" || { echo "$(RED)[ERROR]$(NC) GITHUB_GH_TOKEN is invalid or cannot push to ${GH_REPO_SLUG} (gh api did not answer permissions.push=true)"; exit 1; }; \
	echo "$(GREEN)[SUCCESS]$(NC) GITHUB_GH_TOKEN is valid and can push to ${GH_REPO_SLUG}"
# 2. crates.io, as the workflow sees it: release.yml publishes with the Actions secret
#    CARGO_REGISTRY_TOKEN. GitHub never reveals a secret's value, so this proves the secret EXISTS -
#    not that it is valid. (Listing secrets needs admin rights on the repository.)
	@names=`GH_TOKEN="$$GITHUB_GH_TOKEN" ${GH} secret list --repo ${GH_REPO_SLUG} --json name --jq '.[].name' 2>/dev/null` \
		|| { echo "$(RED)[ERROR]$(NC) cannot list the Actions secrets of ${GH_REPO_SLUG} - GITHUB_GH_TOKEN needs admin rights there"; exit 1; }; \
	printf '%s\n' "$$names" | grep -Fxq CARGO_REGISTRY_TOKEN || { echo "$(RED)[ERROR]$(NC) the Actions secret CARGO_REGISTRY_TOKEN is missing - release.yml could not publish; add it under Settings > Secrets and variables > Actions"; exit 1; }; \
	echo "$(GREEN)[SUCCESS]$(NC) the Actions secret CARGO_REGISTRY_TOKEN exists"
# 3. crates.io, the devops token (account_cargo.env, also what ondewo_publish_crate uploads with):
#    /api/v1/me accepts only browser sessions, so a LIVE API token is answered "this action can only
#    be performed on the crates.io website", and a revoked or mistyped one "authentication failed".
#    That proves the token authenticates - not its scopes, and not that it equals the Actions secret.
#    The header goes to curl on stdin (-H @-), so the token is never on an argv.
	@if [ -z "$$CARGO_REGISTRY_TOKEN" ] || [ "$$CARGO_REGISTRY_TOKEN" = "ENTER_HERE_YOUR_CARGO_REGISTRY_TOKEN" ]; then \
		echo "$(YELLOW)[WARN]$(NC) CARGO_REGISTRY_TOKEN is not in the environment - only the Actions secret's presence was checked"; \
	else \
		answer=`printf 'Authorization: %s\n' "$$CARGO_REGISTRY_TOKEN" | curl -sS -A "${CRATES_IO_USER_AGENT}" -H @- https://crates.io/api/v1/me`; \
		case "$$answer" in \
			*"can only be performed on the crates.io website"*) echo "$(GREEN)[SUCCESS]$(NC) CARGO_REGISTRY_TOKEN authenticates at crates.io";; \
			*) echo "$(RED)[ERROR]$(NC) crates.io refused CARGO_REGISTRY_TOKEN (revoked or mistyped?) - renew it in ${DEVOPS_ACCOUNT_GIT} and in the Actions secret"; exit 1;; \
		esac; \
	fi

release_verify: ## Read-only: the release is complete - tag on origin, non-empty GitHub release, crate served by crates.io
	@$(LOAD_DEVOPS_CREDENTIALS); $(SUBMAKE) --no-print-directory release_verify_checks

release_verify_checks: ## The checks of release_verify, with the credentials already in the environment
	@status=0; git ls-remote --exit-code --tags origin "refs/tags/${ONDEWO_SIP_VERSION}" >/dev/null || status=$$?; \
	test $$status -eq 0 || { echo "$(RED)[ERROR]$(NC) tag ${ONDEWO_SIP_VERSION} is not on origin (git ls-remote exit $$status)"; exit 1; }; \
	echo "$(GREEN)[SUCCESS]$(NC) tag ${ONDEWO_SIP_VERSION} is on origin"
	@body=`GH_TOKEN="$$GITHUB_GH_TOKEN" ${GH} release view "${ONDEWO_SIP_VERSION}" --repo ${GH_REPO_SLUG} --json body --jq .body`; \
	test -n "`printf '%s' "$$body" | tr -d '[:space:]'`" || { echo "$(RED)[ERROR]$(NC) the GitHub release ${ONDEWO_SIP_VERSION} is missing or has an empty body"; exit 1; }; \
	echo "$(GREEN)[SUCCESS]$(NC) the GitHub release ${ONDEWO_SIP_VERSION} has release notes"
	$(MAKE) release_verify_crate

release_verify_crate: ## Read-only: wait for the tag's release.yml run to succeed and for crates.io to serve the version
# The run appears a few seconds after the tag push; for a tag push GitHub reports the tag as the
# run's branch. `gh run watch --exit-status` returns at once for a finished run.
	@run_id=""; attempt=0; \
	while [ $$attempt -lt ${VERIFY_ATTEMPTS} ]; do \
		attempt=$$((attempt + 1)); \
		run_id=`GH_TOKEN="$$GITHUB_GH_TOKEN" ${GH} run list --repo ${GH_REPO_SLUG} --workflow release.yml --event push --branch "${ONDEWO_SIP_VERSION}" --limit 1 --json databaseId --jq '.[0].databaseId // empty'`; \
		test -n "$$run_id" && break; \
		echo "$(BLUE)[INFO]$(NC) no release.yml run for tag ${ONDEWO_SIP_VERSION} yet ($$attempt/${VERIFY_ATTEMPTS})"; sleep ${VERIFY_INTERVAL_S}; \
	done; \
	test -n "$$run_id" || { echo "$(RED)[ERROR]$(NC) release.yml never started for tag ${ONDEWO_SIP_VERSION}"; exit 1; }; \
	echo "$(BLUE)[INFO]$(NC) watching release.yml run $$run_id"; \
	GH_TOKEN="$$GITHUB_GH_TOKEN" ${GH} run watch "$$run_id" --repo ${GH_REPO_SLUG} --exit-status --interval ${VERIFY_INTERVAL_S} >/dev/null \
		|| { echo "$(RED)[ERROR]$(NC) release.yml run $$run_id for tag ${ONDEWO_SIP_VERSION} failed: ${GH_REPO}/actions/runs/$$run_id"; exit 1; }; \
	echo "$(GREEN)[SUCCESS]$(NC) release.yml run $$run_id succeeded"
	@attempt=0; \
	while [ $$attempt -lt ${VERIFY_ATTEMPTS} ]; do \
		attempt=$$((attempt + 1)); \
		answer=`curl -fsS -A "${CRATES_IO_USER_AGENT}" "${CRATES_IO_VERSION_URL}" 2>/dev/null` || answer=""; \
		if printf '%s' "$$answer" | grep -Fq '"num":"${ONDEWO_SIP_VERSION}"' && printf '%s' "$$answer" | grep -Fq '"yanked":false'; then \
			echo "$(GREEN)[SUCCESS]$(NC) crates.io serves ${CRATE_NAME} ${ONDEWO_SIP_VERSION}"; exit 0; \
		fi; \
		echo "$(BLUE)[INFO]$(NC) crates.io does not serve ${CRATE_NAME} ${ONDEWO_SIP_VERSION} yet ($$attempt/${VERIFY_ATTEMPTS})"; sleep ${VERIFY_INTERVAL_S}; \
	done; \
	echo "$(RED)[ERROR]$(NC) crates.io still does not serve ${CRATE_NAME} ${ONDEWO_SIP_VERSION}: ${CRATES_IO_VERSION_URL}"; exit 1

create_release_branch: ## Create Release Branch and push it to origin
	git checkout -b "release/${ONDEWO_SIP_VERSION}"
	git push -u origin "release/${ONDEWO_SIP_VERSION}"

create_release_tag: ## Create Release Tag and push it to origin
	git tag -a ${ONDEWO_SIP_VERSION} -m "release/${ONDEWO_SIP_VERSION}"
	git push origin ${ONDEWO_SIP_VERSION}

########################################################
#		GITHUB

release_notes: ## Print the RELEASE.md section of ONDEWO_SIP_VERSION (exactly what the GitHub release body is)
	@${RELEASE_NOTES_SLICE}

check_release_notes: ## Assert RELEASE.md carries a non-empty section for ONDEWO_SIP_VERSION
# `gh release create` with empty notes succeeds and publishes an EMPTY release, so an entry that was
# forgotten - or a heading whose wording drifted away from RELEASE_NOTES_SLICE - is otherwise only
# noticed by whoever reads the release page afterwards. The heading and the separator do not count.
	@${RELEASE_NOTES_SLICE} | grep -v -E '^#|^\*{5}' | grep -q '[^[:space:]]' || { \
		echo "$(RED)[ERROR]$(NC) RELEASE.md has no non-empty '## ${RELEASE_NOTES_HEADING} ${ONDEWO_SIP_VERSION}' section"; \
		echo "        The GitHub release would be created with empty notes - add the entry first."; \
		exit 1; }
	@echo "$(GREEN)[SUCCESS]$(NC) RELEASE.md has release notes for ${ONDEWO_SIP_VERSION}"

build_gh_release: check_release_notes ## Create the GitHub release (gh authenticates with GITHUB_GH_TOKEN for this call only)
# The notes go to gh on stdin, so their quotes and backticks never meet a shell.
	@test -n "$$GITHUB_GH_TOKEN" || { echo "$(RED)[ERROR]$(NC) GITHUB_GH_TOKEN is not set"; exit 1; }
	${RELEASE_NOTES_SLICE} | GH_TOKEN="$$GITHUB_GH_TOKEN" ${GH} release create "${ONDEWO_SIP_VERSION}" --repo ${GH_REPO_SLUG} --verify-tag --notes-file - -t "Release ${ONDEWO_SIP_VERSION}"

########################################################
#		CRATES.IO

check_crate_metadata: ## Assert Cargo.toml carries everything crates.io requires of a publishable crate
# `cargo publish --dry-run` already rejects an empty description/license/repository and a missing
# readme file, but it says nothing about keywords/categories (crates.io only recommends those) and
# it cannot see the server-side size limit. Keeping the whole list in one credential-free target
# means CI exercises every one of them on every push, instead of discovering them at release time.
	@for field in description license repository readme; do \
		grep -Eq "^$$field = \"[^\"]+\"" Cargo.toml || { echo "$(RED)[ERROR]$(NC) Cargo.toml carries no non-empty '$$field' - crates.io refuses the upload without it"; exit 1; }; \
	done
	@grep -Eq '^keywords = \[[^]]+\]' Cargo.toml || { echo "$(RED)[ERROR]$(NC) Cargo.toml carries no 'keywords' - the crate would be unfindable on crates.io"; exit 1; }
	@grep -Eq '^categories = \[[^]]+\]' Cargo.toml || { echo "$(RED)[ERROR]$(NC) Cargo.toml carries no 'categories' - the crate would be unfindable on crates.io"; exit 1; }
	@! grep -Eq '^[[:space:]]*publish[[:space:]]*=[[:space:]]*false' Cargo.toml || { echo "$(RED)[ERROR]$(NC) Cargo.toml sets 'publish = false' - the crate cannot be published at all"; exit 1; }
	@readme=`sed -n 's|^readme = "\(.*\)"|\1|p' Cargo.toml | head -n 1`; \
		test -s "$$readme" || { echo "$(RED)[ERROR]$(NC) the readme '$$readme' declared in Cargo.toml is missing or empty - crates.io renders it as the crate page"; exit 1; }
	@echo "$(GREEN)[SUCCESS]$(NC) Cargo.toml carries every field crates.io requires"

publish_crate_dry_run: check_crate_metadata ## Credential-free run of the whole packaging path, minus the upload (this is what CI runs)
# Everything the crates.io release does except the upload itself, and none of it needs a token.
# --allow-dirty throughout because this also runs over a working tree with freshly generated stubs;
# the real publish_crate deliberately has no such flag and refuses a dirty tree.
#
# First the exact file list crates.io would receive. The readme needs no assertion here: cargo
# always packages the file named by `readme`, even against an exclude entry, and
# check_crate_metadata has already proven that file exists and is non-empty.
	@echo "$(BLUE)[INFO]$(NC) Files that would be published:"
	${CARGO} package --list --allow-dirty --locked
# `cargo publish --dry-run` builds its tarball in a scratch directory and does not leave it behind,
# so the size limit is measured on the one `cargo package` writes. --no-verify keeps this step to
# the tarball alone (seconds); the compile that proves the packaged copy builds is the dry run below.
	${CARGO} package --no-verify --allow-dirty --locked
# Name the tarball from the manifest rather than globbing target/package: a cached build directory
# can still hold the .crate of an earlier version, and a glob would happily measure that one.
	@crate_file="target/package/`sed -n 's|^name = "\(.*\)"|\1|p' Cargo.toml | head -n 1`-`sed -n 's|^version = "\(.*\)"|\1|p' Cargo.toml | head -n 1`.crate"; \
		test -f "$$crate_file" || { echo "$(RED)[ERROR]$(NC) cargo package produced no $$crate_file"; exit 1; }; \
		crate_bytes=`wc -c < "$$crate_file" | tr -d ' '`; \
		test "$$crate_bytes" -le ${CRATE_MAX_BYTES} || { echo "$(RED)[ERROR]$(NC) $$crate_file is $$crate_bytes bytes, over the crates.io limit of ${CRATE_MAX_BYTES} - trim Cargo.toml's exclude list"; exit 1; }; \
		echo "$(GREEN)[SUCCESS]$(NC) $$crate_file is $$crate_bytes bytes (crates.io limit: ${CRATE_MAX_BYTES})"
	${CARGO} publish --dry-run --allow-dirty --locked

publish_crate: check_crate_metadata ## Publish the crate to crates.io (needs CARGO_REGISTRY_TOKEN)
# cargo reads CARGO_REGISTRY_TOKEN from the environment it was started with, so the token
# never appears on a command line or in the build log. The guard below reads it through the SHELL
# ($$VAR), not through make ($(VAR)), so the token is not interpolated into the recipe text either.
	@test -n "$$CARGO_REGISTRY_TOKEN" -a "$$CARGO_REGISTRY_TOKEN" != "ENTER_HERE_YOUR_CARGO_REGISTRY_TOKEN" || { echo "$(RED)[ERROR]$(NC) CARGO_REGISTRY_TOKEN is not set - create one at https://crates.io/settings/tokens, or run 'make ondewo_publish_crate' to take it from the devops-accounts repo"; exit 1; }
	@echo "$(BLUE)[INFO]$(NC) Publishing ondewo-sip-client ${ONDEWO_SIP_VERSION} to crates.io ..."
	${CARGO_PUBLISH} publish --locked
	@echo "$(GREEN)[SUCCESS]$(NC) Published to crates.io"

package_crate: ## Package the crate locally (the same artifact `make publish_crate` uploads)
	${CARGO} package --locked

########################################################
#		DEVOPS-ACCOUNTS

# One shell, so the credentials exist only in that shell's environment and the clone is gone
# BEFORE `$(MAKE) release` starts: spc first (no credentials needed), then load, then delete. The
# EXIT/INT/TERM trap deletes our clone on every failure path too. The clone lives in a fresh
# `mktemp -d` outside this repository, so no docker mount or build context can ever contain it.
# Anchored greps, exported with `set -a` into the environment (never `make release NAME=<value>`,
# which would put the token on make's argv, visible to every user in /proc/<pid>/cmdline).
ondewo_release: spc ## Release with credentials from devops-accounts repo (honours DEVOPS_ACCOUNT_DIR)
	@set -eu; \
	owned=""; \
	if [ -n "${DEVOPS_ACCOUNT_DIR}" ] && [ -d "${DEVOPS_ACCOUNT_DIR}" ]; then \
		accounts="${DEVOPS_ACCOUNT_DIR}"; \
		echo "$(BLUE)[INFO]$(NC) Using the devops accounts in DEVOPS_ACCOUNT_DIR (owned by the caller, not deleted here)"; \
	else \
		owned=`mktemp -d`; \
		trap 'rm -rf "$$owned"' EXIT INT TERM; \
		accounts="$$owned/${DEVOPS_ACCOUNT_GIT}"; \
		git clone --quiet --depth 1 ${DEVOPS_ACCOUNT_REPO} "$$accounts"; \
	fi; \
	set -a; \
	eval "`grep -h -E '^GITHUB_GH_TOKEN=' "$$accounts/account_github.env"`"; \
	if [ -f "$$accounts/account_cargo.env" ]; then eval "`grep -h -E '^CARGO_REGISTRY_TOKEN=' "$$accounts/account_cargo.env"`"; fi; \
	set +a; \
	if [ -n "$$owned" ]; then rm -rf "$$owned"; echo "$(BLUE)[INFO]$(NC) Deleted the devops-accounts clone"; fi; \
	$(SUBMAKE) release

ondewo_publish_crate: ## Manual crates.io fallback: publish with the token from the devops-accounts repo (honours DEVOPS_ACCOUNT_DIR)
	@set -eu; \
	owned=""; \
	if [ -n "${DEVOPS_ACCOUNT_DIR}" ] && [ -d "${DEVOPS_ACCOUNT_DIR}" ]; then \
		accounts="${DEVOPS_ACCOUNT_DIR}"; \
	else \
		owned=`mktemp -d`; \
		trap 'rm -rf "$$owned"' EXIT INT TERM; \
		accounts="$$owned/${DEVOPS_ACCOUNT_GIT}"; \
		git clone --quiet --depth 1 ${DEVOPS_ACCOUNT_REPO} "$$accounts"; \
	fi; \
	set -a; \
	eval "`grep -h -E '^CARGO_REGISTRY_TOKEN=' "$$accounts/account_cargo.env"`"; \
	set +a; \
	if [ -n "$$owned" ]; then rm -rf "$$owned"; echo "$(BLUE)[INFO]$(NC) Deleted the devops-accounts clone"; fi; \
	$(SUBMAKE) publish_crate

spc: ## Refuse a version that already exists: release branch or tag (origin and local), or the crate version on crates.io
# Anchored, exact ref names: `git ls-remote --exit-code` exits 2 when the ref is absent, 0 when it
# exists and anything else when origin could not be asked - which is an error, not a pass.
	@status=0; git ls-remote --exit-code --heads origin "refs/heads/release/${ONDEWO_SIP_VERSION}" >/dev/null || status=$$?; \
	case $$status in \
		0) echo "-- Test 1: Branch release/${ONDEWO_SIP_VERSION} exists on origin!!"; exit 1;; \
		2) ;; \
		*) echo "-- Test 1: cannot list the branches of origin (git ls-remote exit $$status)"; exit 1;; \
	esac; \
	if git rev-parse -q --verify "refs/heads/release/${ONDEWO_SIP_VERSION}" >/dev/null; then echo "-- Test 1: Branch release/${ONDEWO_SIP_VERSION} exists locally!!"; exit 1; fi; \
	echo "-- Test 1: Branch is fine"
	@status=0; git ls-remote --exit-code --tags origin "refs/tags/${ONDEWO_SIP_VERSION}" >/dev/null || status=$$?; \
	case $$status in \
		0) echo "-- Test 2: Tag ${ONDEWO_SIP_VERSION} exists on origin!!"; exit 1;; \
		2) ;; \
		*) echo "-- Test 2: cannot list the tags of origin (git ls-remote exit $$status)"; exit 1;; \
	esac; \
	if git rev-parse -q --verify "refs/tags/${ONDEWO_SIP_VERSION}" >/dev/null; then echo "-- Test 2: Tag ${ONDEWO_SIP_VERSION} exists locally!!"; exit 1; fi; \
	echo "-- Test 2: Tag is fine"
	@code=`curl -sS -o /dev/null -w '%{http_code}' -A "${CRATES_IO_USER_AGENT}" "${CRATES_IO_VERSION_URL}"` || code=error; \
	case $$code in \
		404) echo "-- Test 3: ${CRATE_NAME} ${ONDEWO_SIP_VERSION} is not on crates.io yet";; \
		200) echo "-- Test 3: ${CRATE_NAME} ${ONDEWO_SIP_VERSION} is already on crates.io!!"; exit 1;; \
		*) echo "-- Test 3: cannot ask crates.io about ${CRATE_NAME} ${ONDEWO_SIP_VERSION} (HTTP $$code)"; exit 1;; \
	esac
