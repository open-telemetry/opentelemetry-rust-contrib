# Contributing to Opentelemetry Rust contrib repo

The Rust special interest group (SIG) meets weekly on Tuesdays at 9 AM Pacific
Time. The meeting is subject to change depending on contributors'
availability. Check the [OpenTelemetry community
calendar](https://github.com/open-telemetry/community?tab=readme-ov-file#calendar)
for specific dates and for Zoom meeting links. "OTel Rust SIG" is the name of
meeting for this group.

Meeting notes are available as a public [Google
doc](https://docs.google.com/document/d/12upOzNk8c3SFTjsL6IRohCWMgzLKoknSCOOdMakbWo4/edit).
If you have trouble accessing the doc, please get in touch on the
[#otel-rust](https://cloud-native.slack.com/archives/C03GDP0H023)
channel on CNCF Slack. If you are new to the CNCF Slack community,
you can [create an account](https://slack.cncf.io/).

The meeting is open for all to join. We invite everyone to join our meeting,
regardless of your experience level. Whether you're a seasoned OpenTelemetry
developer, just starting your journey, or simply curious about the work we do,
you're more than welcome to participate!

## Pull Requests

### How to Send Pull Requests

Everyone is welcome to contribute code to `opentelemetry-rust-contrib` via
GitHub pull requests (PRs).

```sh
git clone https://github.com/open-telemetry/opentelemetry-rust-contrib
```

Enter the newly created directory and add your fork as a new remote:

```sh
git remote add <YOUR_FORK> git@github.com:<YOUR_GITHUB_USERNAME>/opentelemetry-rust-contrib
```

Check out a new branch, make modifications, run linters and tests, and
push the branch to your fork:

```sh
$ git checkout -b <YOUR_BRANCH_NAME>
# edit files
$ git add -p
$ git commit
$ git push <YOUR_FORK> <YOUR_BRANCH_NAME>
```

Open a pull request against the main
[opentelemetry-rust-contrib](https://github.com/open-telemetry/opentelemetry-rust-contrib)
repo.

Your pull request should be named according to the
[conventional commits](https://www.conventionalcommits.org/en/v1.0.0/) standard. This ensures that
when the PR is squashed into `main`, the resulting commit message is consistent and makes it easier
for us to generate a changelog  standard.

> **Note**
> It is recommended to run [pre-commit script](precommit.sh) from the root of
the repo to catch any issues locally.

### How to Receive Comments

- If the PR is not ready for review, please put `[WIP]` in the title or mark it
  as [`draft`](https://github.blog/2019-02-14-introducing-draft-pull-requests/).
- Make sure CLA is signed and all required CI checks are clear.
- Submit small, focused PRs addressing a single concern/issue.
- Make sure the PR title reflects the contribution.
- Write a summary that helps understand the change.
- Include usage examples in the summary, where applicable.
- Include benchmarks (before/after) in the summary, for contributions that are
  performance enhancements.

### How to Get PRs Merged

A PR is considered to be **ready to merge** when:

- It has received approval from
  [Approvers](https://github.com/open-telemetry/community/blob/main/community-membership.md#approver).
  /
  [Maintainers](https://github.com/open-telemetry/community/blob/main/community-membership.md#maintainer).
- Major feedbacks are resolved.

Any Maintainer can merge the PR once it is **ready to merge**. Note, that some
PRs may not be merged immediately if the repo is in the process of a release and
the maintainers decided to defer the PR to the next release train. Also,
maintainers may decide to wait for more than one approval for certain PRs,
particularly ones that are affecting multiple areas, or topics that may warrant
more discussion.

## Component Ownership

Each crate in this repo is expected to have one or more **component owners**
listed in [`.github/component_owners.yml`](.github/component_owners.yml).
Owners are responsible for the long-term health of a single component; repo
maintainers and approvers cover the repo as a whole.

### Responsibilities

Owners are expected to:

- Respond to issues and PRs filed against the component in a reasonable
  timeframe.
- Keep the crate working as the upstream `opentelemetry` SDK and broader
  ecosystem evolve.
- Help triage new contributions and reviews against the crate.

In practice, owners are automatically requested as reviewers on PRs that
touch their crate and pinged on issues that select the crate in the issue
template, so the main operational duty is watching GitHub notifications and
responding. During regular upstream `opentelemetry` releases, the maintainer
cutting the release usually handles trivial crate updates (for example,
bumping the `opentelemetry` dependency); owners are looped in for
non-trivial cases such as API breaks. Broader changes that affect components
are discussed at the OTel Rust SIG meeting and on the `#otel-rust` Slack
channel.

### Eligibility

Component owners must be
[OpenTelemetry Members](https://github.com/open-telemetry/community/blob/main/guides/contributor/membership.md#member).
If you aren't one yet, the linked guide explains how to apply, and existing
repo maintainers are happy to sponsor active contributors.

Repo maintainers and approvers may also be listed as component owners.

### Becoming or stepping down as an owner

To volunteer as an owner for an existing crate, open a PR adding your
GitHub username to that crate's entry in
[`.github/component_owners.yml`](.github/component_owners.yml). Two or
more owners per crate is preferred so coverage doesn't depend on a single
person.

If you can no longer maintain a component, open a PR removing yourself.
There is no formal inactivity policy today; if a crate's listed owners stop
responding for an extended period, maintainers may seek replacements or, as
a last resort, mark the crate for removal (see issue
[#609](https://github.com/open-telemetry/opentelemetry-rust-contrib/issues/609)
for a recent example).

## Adding a New Component

This repo hosts community-contributed exporters, instrumentation libraries,
resource detectors, propagators, and other extensions that don't belong in the
core [opentelemetry-rust](https://github.com/open-telemetry/opentelemetry-rust)
SDK.

**Before writing a PR, open an issue** describing what you want to add and
why it belongs here. This lets maintainers and the community weigh in on
fit, naming, and ownership before you invest time in the implementation.
The [#otel-rust](https://cloud-native.slack.com/archives/C03GDP0H023)
channel on CNCF Slack is also a good place for open-ended questions; if
you are new to the CNCF Slack community, you can
[create an account](https://slack.cncf.io/).

Once maintainers agree the component belongs here, the sections below
describe the extra requirements that apply on top of the regular PR process
(see [Pull Requests](#pull-requests)). For a worked example, look at the
[opentelemetry-etw-traces](https://github.com/open-telemetry/opentelemetry-rust-contrib/tree/main/opentelemetry-etw-traces)
crate and the PR that added it ([#562](https://github.com/open-telemetry/opentelemetry-rust-contrib/pull/562)).

### 1. Pick a name

Crate names follow the pattern `opentelemetry-<kind>-<name>` (for example
`opentelemetry-exporter-geneva`, `opentelemetry-instrumentation-tower`,
`opentelemetry-resource-detectors`). For new crates, the folder at the repo
root is the same as the crate name. See existing top-level folders for
established kinds and naming.

### 2. Find a component owner

Every component must have at least one owner; see
[Component Ownership](#component-ownership) above for what owners are
expected to do and who qualifies. You can own a component you contribute
yourself, or recruit someone from the community.

### 3. Add the crate files

A new component crate needs, at minimum:

- `Cargo.toml`. Copy the `[package]` block from an existing crate (for
  example, [opentelemetry-etw-logs/Cargo.toml](opentelemetry-etw-logs/Cargo.toml))
  and adjust the metadata. Start the version at `0.1.0`, use
  `license = "Apache-2.0"`, and inherit lints via `[lints] workspace = true`.
- `README.md` describing what the component does, how to install it, and a
  minimal usage example. Include a status table near the top with `Stability`
  and `Owners` rows — see
  [opentelemetry-etw-logs/README.md](opentelemetry-etw-logs/README.md) for
  the format. Stability is one of: `alpha` (early, breaking changes likely),
  `beta` (usable, API may still change), or `stable` (committed API).
- `CHANGELOG.md` with a `vNext` heading at the top for future entries.
- `src/lib.rs` with the implementation. The crate must build on every
  platform CI runs on, even if its functionality is OS-specific — gate
  platform-specific code with `#[cfg(...)]` so the crate compiles cleanly
  elsewhere.

### 4. Add to workspace and automation

Three places need updating in the same PR so the crate is picked up by
tooling and automation:

- Root [`Cargo.toml`](Cargo.toml): add the crate folder to `members`.
- [`.github/component_owners.yml`](.github/component_owners.yml): add an
  entry mapping the crate folder to a list of GitHub usernames. This file
  is the single source of truth for both
  [PR-reviewer assignment](.github/workflows/assign-reviewers.yml) and
  [issue owner pings](.github/workflows/ping-component-owners.yml).
- Issue templates: add the crate to the component dropdown in both
  [`.github/ISSUE_TEMPLATE/BUG-REPORT.yml`](.github/ISSUE_TEMPLATE/BUG-REPORT.yml)
  and
  [`.github/ISSUE_TEMPLATE/FEATURE-REQUEST.yml`](.github/ISSUE_TEMPLATE/FEATURE-REQUEST.yml).

If the component needs platform-specific integration tests or a separate
toolchain that the default CI job doesn't cover, also add a job (or extend
an existing one) in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml).

After the PR is merged, the first release of the crate is cut by a
maintainer. To request a release, open an issue or ping in the
[#otel-rust](https://cloud-native.slack.com/archives/C03GDP0H023) channel
on CNCF Slack.

## Design Choices

As with other OpenTelemetry clients, opentelemetry-rust follows the
[opentelemetry-specification](https://github.com/open-telemetry/opentelemetry-specification).

It's especially valuable to read through the [library
guidelines](https://github.com/open-telemetry/opentelemetry-specification/blob/master/specification/library-guidelines.md).

### Focus on Capabilities, Not Structure Compliance

OpenTelemetry is an evolving specification, one where the desires and
use cases are clear, but the method to satisfy those uses cases are
not.

As such, Contributions should provide functionality and behavior that
conforms to the specification, but the interface and structure is
flexible.

It is preferable to have contributions follow the idioms of the
language rather than conform to specific API names or argument
patterns in the spec.

For a deeper discussion, see:
<https://github.com/open-telemetry/opentelemetry-specification/issues/165>

### Error Handling

Currently, the Opentelemetry Rust SDK has two ways to handle errors. In the situation where errors are not allowed to return. One should call global error handler to process the errors. Otherwise, one should return the errors.

The Opentelemetry Rust SDK comes with an error type `openetelemetry::Error`. For different function, one error has been defined. All error returned by trace module MUST be wrapped in `opentelemetry::trace::TraceError`. All errors returned by metrics module MUST be wrapped in `opentelemetry::metrics::MetricsError`.

For users that want to implement their own exporters. It's RECOMMENDED to wrap all errors from the exporter into a crate-level error type, and implement `ExporterError` trait.

### Priority of configurations

OpenTelemetry supports multiple ways to configure the API, SDK and other components. When the same setting is provided through more than one mechanism, code-based configuration (e.g. `with_xxx` builder methods) takes precedence over environment variables. New components added under this repo MUST follow this priority.

## Style Guide

- Run `cargo clippy --all` - this will catch common mistakes and improve
your Rust code
- Run `cargo fmt` - this will find and fix code formatting
issues.

## Testing and Benchmarking

- Run `cargo test --all` - this will execute code and doc tests for all
projects in this workspace.
- Run `cargo bench` - this will run benchmarks to show performance
- Run `cargo bench` - this will run benchmarks to show performance
regressions
