# Releasing OpenTelemetry Rust Contrib

The primary audience for this is the SIG Maintainers. It provides the list of steps for how to release the crates and the
considerations to make before releasing the crate. It may provide use to consumers of the crate if/when we develop a
release cadence.

## Release cadence

There is no established cadence for the OpenTelemetry Rust Contrib crates. Each crate in this repository may be released
independently as needed. The balance is required between too many breaking changes in a single release, and since we
have instability flipping between implementations across 0.x releases.

## Considerations

A draft PR can be created, but before releasing consider the following:

* Are there any pending pull requests which should be included in the next release?
  * Are they blockers?
* Are there any unresolved issues which should be resolved before the next release? Check the release [blockers milestone](https://github.com/open-telemetry/opentelemetry-rust-contrib/milestones) for every release
* For crates that depend on `opentelemetry`, `opentelemetry-sdk`, or other core crates, ensure the contrib crate is compatible with the latest released versions of those core crates.
* Bring it up at a SIG meeting, this can usually get some of these questions answered sooner than later. It will also
  help establish a person to perform the release. Ideally this can be someone different each time to ensure that the
  process is documented.

## Steps to Release

1. Create a release PR

* For each crate being released
  * Bump appropriate version
  * Update change logs to reflect release version.
  * Update dependency versions on core `opentelemetry-*` crates as necessary
* If there's a large enough set of changes, consider writing a migration guide.

2. Merge the PR

* Get reviews from other Maintainers
* Ensure that there haven't been any interfering PRs

3. Tag the release commit based on the [tagging convention](#tagging-convention). It should usually be a bump on minor version before 1.0
4. Create Github Release
5. [Publish](#publishing-crates) to crates.io using the version as of the release commit
6. Post to [#otel-rust](https://cloud-native.slack.com/archives/C03GDP0H023) on CNCF Slack.

[Publish.sh](./publish.sh) may be used to automate steps 3 and 5.

## Tagging Convention

For each crate: it should be `<crate-name>-<version>` `<version>` being the simple `X.Y.Z`.
For example:

```sh
git tag -a opentelemetry-etw-logs-0.10.0 -m "opentelemetry-etw-logs 0.10.0 release"
git push origin opentelemetry-etw-logs-0.10.0
```

## Publishing Crates

For now we use the [basic procedure](https://doc.rust-lang.org/cargo/reference/publishing.html) from crates.io.

Follow this for each crate as necessary.

For any new crates remember to add open-telemetry/rust-maintainers to the list of owners.
