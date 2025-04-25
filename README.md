# OpenTelemetry Rust Contrib

The Rust [OpenTelemetry](https://opentelemetry.io/) implementation of contrib components.

[![LICENSE](https://img.shields.io/crates/l/opentelemetry)](./LICENSE)
[![GitHub Actions CI](https://github.com/open-telemetry/opentelemetry-rust-contrib/workflows/CI/badge.svg)](https://github.com/open-telemetry/opentelemetry-rust-contrib/actions?query=workflow%3ACI+branch%3Amain)
[![codecov](https://codecov.io/gh/open-telemetry/opentelemetry-rust-contrib/branch/main/graph/badge.svg)](https://codecov.io/gh/open-telemetry/opentelemetry-rust-contrib)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/open-telemetry/opentelemetry-rust-contrib/badge)](https://scorecard.dev/viewer/?uri=github.com/open-telemetry/opentelemetry-rust-contrib)
[![Slack](https://img.shields.io/badge/slack-@cncf/otel/rust-brightgreen.svg?logo=slack)](https://cloud-native.slack.com/archives/C03GDP0H023)

[Website](https://opentelemetry.io/) |
[Slack](https://cloud-native.slack.com/archives/C03GDP0H023) |

## Overview

This repo is intended to provide helpful libraries and standalone
OpenTelemetry-based utilities that don't fit the primary scope of the
[OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust)
project.

*Compiler support: [requires `rustc` 1.70+][msrv]*

[msrv]: #supported-rust-versions

## Getting Started

Check individual folders for usage guidelines and examples.

## Supported Rust Versions

OpenTelemetry is built against the latest stable release. The minimum supported
version is 1.75. The current OpenTelemetry version is not guaranteed to build
on Rust versions earlier than the minimum supported version.

The current stable Rust compiler and the three most recent minor versions
before it will always be supported. For example, if the current stable compiler
version is 1.49, the minimum supported version will not be increased past 1.46,
three minor versions prior. Increasing the minimum supported compiler version
is not considered a semver breaking change as long as doing so complies with
this policy.

## Contributing

See the [contributing file](CONTRIBUTING.md).

The Rust special interest group (SIG) meets weekly on Tuesdays at 9 AM Pacific
Time. The meeting is subject to change depending on contributors' availability.
Check the [OpenTelemetry community
calendar](https://github.com/open-telemetry/community?tab=readme-ov-file#calendar)
for specific dates and for Zoom meeting links. "OTel Rust SIG" is the name of
meeting for this group.

Meeting notes are available as a public [Google
doc](https://docs.google.com/document/d/12upOzNk8c3SFTjsL6IRohCWMgzLKoknSCOOdMakbWo4/edit).
If you have trouble accessing the doc, please get in touch on
[Slack](https://cloud-native.slack.com/archives/C03GDP0H023).

The meeting is open for all to join. We invite everyone to join our meeting,
regardless of your experience level. Whether you're a seasoned OpenTelemetry
developer, just starting your journey, or simply curious about the work we do,
you're more than welcome to participate!

## Approvers and Maintainers

### Maintainers

* [Cijo Thomas](https://github.com/cijothomas)
* [Harold Dost](https://github.com/hdost)
* [Julian Tescher](https://github.com/jtescher)
* [Lalit Kumar Bhasin](https://github.com/lalitb)
* [Zhongyang Wu](https://github.com/TommyCpp)

### Approvers

* [Shaun Cox](https://github.com/shaun-cox)

### Emeritus

* [Dirkjan Ochtman](https://github.com/djc)
* [Jan Kühle](https://github.com/frigus02)
* [Isobel Redelmeier](https://github.com/iredelmeier)
* [Mike Goldsmith](https://github.com/MikeGoldsmith)

### Thanks to all the people who have contributed

[![contributors](https://contributors-img.web.app/image?repo=open-telemetry/opentelemetry-rust-contrib)](https://github.com/open-telemetry/opentelemetry-rust-contrib/graphs/contributors)