use opentelemetry_sdk::logs::TraceContext;
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};
use tracelogging_dynamic as tld;

pub(crate) fn populate_part_a(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) {
    if let Some(trace_context) = log_record.trace_context() {
        populate_part_a_from_context(event, resource, trace_context, field_tag);
    } else {
        populate_part_a_from_record(event, resource, field_tag);
    }

    populate_time(event, log_record, field_tag);
}

fn populate_part_a_from_record(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    field_tag: u32,
) {
    const COUNT_TIME: u8 = 1u8;

    let field_count = COUNT_TIME + has_ext_cloud(resource);

    event.add_struct("PartA", field_count, field_tag);

    populate_resource(resource, event, field_tag);
}

fn populate_part_a_from_context(
    event: &mut tld::EventBuilder,
    resource: &super::Resource,
    trace_context: &TraceContext,
    field_tag: u32,
) {
    const COUNT_TIME: u8 = 1u8;
    const COUNT_EXT_DT: u8 = 1u8;
    let field_count = COUNT_TIME + COUNT_EXT_DT + has_ext_cloud(resource);

    event.add_struct("PartA", field_count, field_tag);

    const EXT_DT_COUNT: u8 = 2u8;
    event.add_struct("ext_dt", EXT_DT_COUNT, field_tag);
    event.add_str8(
        "traceId",
        trace_context.trace_id.to_string(),
        tld::OutType::Default,
        field_tag,
    );
    event.add_str8(
        "spanId",
        trace_context.span_id.to_string(),
        tld::OutType::Default,
        field_tag,
    );

    populate_resource(resource, event, field_tag);
}

fn has_ext_cloud(resource: &super::Resource) -> u8 {
    (resource.cloud_role.is_some() || resource.cloud_role_instance.is_some()) as u8
}

fn populate_resource(resource: &super::Resource, event: &mut tld::EventBuilder, field_tag: u32) {
    let ext_cloud_count =
        resource.cloud_role.is_some() as u8 + resource.cloud_role_instance.is_some() as u8;
    if ext_cloud_count > 0 {
        event.add_struct("ext_cloud", ext_cloud_count, field_tag);
        if let Some(cloud_role) = &resource.cloud_role {
            event.add_str8("role", cloud_role, tld::OutType::Default, field_tag);
        }
        if let Some(cloud_role_instance) = &resource.cloud_role_instance {
            event.add_str8(
                "roleInstance",
                cloud_role_instance,
                tld::OutType::Default,
                field_tag,
            );
        }
    }
}

fn populate_time(
    event: &mut tld::EventBuilder,
    log_record: &opentelemetry_sdk::logs::SdkLogRecord,
    field_tag: u32,
) {
    let event_time: SystemTime = log_record
        .timestamp()
        .or(log_record.observed_timestamp())
        .unwrap_or_else(SystemTime::now);

    event.add_str8(
        "time",
        format_timestamp(event_time).as_str(),
        tld::OutType::Default,
        field_tag,
    );
}

fn format_timestamp(time: SystemTime) -> String {
    let (unix_seconds, nanosecond) = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => (i128::from(duration.as_secs()), duration.subsec_nanos()),
        Err(before_epoch) => {
            let duration = before_epoch.duration();
            if duration.subsec_nanos() == 0 {
                (-i128::from(duration.as_secs()), 0)
            } else {
                (
                    -i128::from(duration.as_secs()) - 1,
                    1_000_000_000 - duration.subsec_nanos(),
                )
            }
        }
    };

    let days = unix_seconds.div_euclid(86_400);
    let seconds_of_day = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;

    let mut timestamp = String::with_capacity(40);
    if year < 0 {
        write!(&mut timestamp, "-{:04}", year.unsigned_abs())
            .expect("writing to a String cannot fail");
    } else if year > 9_999 {
        write!(&mut timestamp, "+{year:04}").expect("writing to a String cannot fail");
    } else {
        write!(&mut timestamp, "{year:04}").expect("writing to a String cannot fail");
    }
    write!(
        &mut timestamp,
        "-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"
    )
    .expect("writing to a String cannot fail");

    if nanosecond != 0 {
        let (fraction, precision) = if nanosecond % 1_000_000 == 0 {
            (nanosecond / 1_000_000, 3)
        } else if nanosecond % 1_000 == 0 {
            (nanosecond / 1_000, 6)
        } else {
            (nanosecond, 9)
        };
        write!(&mut timestamp, ".{fraction:0precision$}").expect("writing to a String cannot fail");
    }
    timestamp.push('Z');

    timestamp
}

fn civil_from_days(days: i128) -> (i128, i128, i128) {
    let shifted_days = days + 719_468;
    let era = shifted_days.div_euclid(146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::format_timestamp;
    use std::time::{Duration, SystemTime};

    #[test]
    fn formats_rfc3339_timestamps_with_auto_si_precision() {
        let cases = [
            (
                SystemTime::UNIX_EPOCH + Duration::from_secs(1_704_164_645),
                "2024-01-02T03:04:05Z",
            ),
            (
                SystemTime::UNIX_EPOCH + Duration::new(1_704_164_645, 123_000_000),
                "2024-01-02T03:04:05.123Z",
            ),
            (
                SystemTime::UNIX_EPOCH + Duration::new(1_704_164_645, 123_456_000),
                "2024-01-02T03:04:05.123456Z",
            ),
            (
                SystemTime::UNIX_EPOCH + Duration::new(1_704_164_645, 123_456_700),
                "2024-01-02T03:04:05.123456700Z",
            ),
        ];

        for (time, expected) in cases {
            assert_eq!(format_timestamp(time), expected);
        }
    }

    #[test]
    fn formats_times_before_the_unix_epoch() {
        let cases = [
            (
                SystemTime::UNIX_EPOCH - Duration::from_nanos(100),
                "1969-12-31T23:59:59.999999900Z",
            ),
            (
                SystemTime::UNIX_EPOCH - Duration::from_millis(100),
                "1969-12-31T23:59:59.900Z",
            ),
            (
                SystemTime::UNIX_EPOCH - Duration::from_secs(86_400),
                "1969-12-31T00:00:00Z",
            ),
        ];

        for (time, expected) in cases {
            assert_eq!(format_timestamp(time), expected);
        }
    }

    #[test]
    fn formats_leap_days_and_extended_years() {
        let cases = [
            (
                SystemTime::UNIX_EPOCH + Duration::from_secs(951_782_400),
                "2000-02-29T00:00:00Z",
            ),
            (
                SystemTime::UNIX_EPOCH + Duration::from_secs(253_402_300_800),
                "+10000-01-01T00:00:00Z",
            ),
        ];

        for (time, expected) in cases {
            assert_eq!(format_timestamp(time), expected);
        }
    }
}
