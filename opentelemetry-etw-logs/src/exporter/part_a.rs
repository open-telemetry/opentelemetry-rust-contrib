use opentelemetry_sdk::logs::TraceContext;
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
        format_timestamp(event_time),
        tld::OutType::Default,
        field_tag,
    );
}

struct Timestamp {
    bytes: [u8; 40],
    len: usize,
}

impl AsRef<[u8]> for Timestamp {
    fn as_ref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

fn write_decimal(mut value: u64, output: &mut [u8]) {
    for digit in output.iter_mut().rev() {
        *digit = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

fn format_timestamp(time: SystemTime) -> Timestamp {
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

    format_unix_timestamp(unix_seconds, nanosecond)
}

fn format_unix_timestamp(unix_seconds: i128, nanosecond: u32) -> Timestamp {
    let days = i64::try_from(unix_seconds.div_euclid(86_400))
        .expect("days from a SystemTime duration fit in i64");
    let seconds_of_day = unix_seconds.rem_euclid(86_400) as u64;
    let (year, month, day) = date_from_days_since_unix_epoch(days);
    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;

    let mut timestamp = Timestamp {
        bytes: [0; 40],
        len: 0,
    };
    if !(0..=9999).contains(&year) {
        timestamp.bytes[0] = if year < 0 { b'-' } else { b'+' };
        timestamp.len = 1;
    }
    let year = year.unsigned_abs();
    let mut width = 4;
    let mut remaining = year / 10_000;
    while remaining > 0 {
        width += 1;
        remaining /= 10;
    }
    write_decimal(
        year,
        &mut timestamp.bytes[timestamp.len..timestamp.len + width],
    );
    timestamp.len += width;
    let offset = timestamp.len;
    timestamp.bytes[offset..offset + 15].copy_from_slice(b"-00-00T00:00:00");
    write_decimal(month as u64, &mut timestamp.bytes[offset + 1..offset + 3]);
    write_decimal(day as u64, &mut timestamp.bytes[offset + 4..offset + 6]);
    write_decimal(hour, &mut timestamp.bytes[offset + 7..offset + 9]);
    write_decimal(minute, &mut timestamp.bytes[offset + 10..offset + 12]);
    write_decimal(second, &mut timestamp.bytes[offset + 13..offset + 15]);
    timestamp.len += 15;

    if nanosecond != 0 {
        let (fraction, precision) = if nanosecond % 1_000_000 == 0 {
            (nanosecond / 1_000_000, 3)
        } else if nanosecond % 1_000 == 0 {
            (nanosecond / 1_000, 6)
        } else {
            (nanosecond, 9)
        };
        timestamp.bytes[timestamp.len] = b'.';
        timestamp.len += 1;
        write_decimal(
            u64::from(fraction),
            &mut timestamp.bytes[timestamp.len..timestamp.len + precision],
        );
        timestamp.len += precision;
    }
    timestamp.bytes[timestamp.len] = b'Z';
    timestamp.len += 1;

    timestamp
}

const fn date_from_days_since_unix_epoch(days: i64) -> (i64, i64, i64) {
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
    use super::{format_timestamp, format_unix_timestamp};
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
            assert_eq!(format_timestamp(time).as_ref(), expected.as_bytes());
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
            assert_eq!(format_timestamp(time).as_ref(), expected.as_bytes());
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
            assert_eq!(format_timestamp(time).as_ref(), expected.as_bytes());
        }
    }

    #[test]
    fn formats_extended_years_before_epoch() {
        let cases = [
            (-62_135_596_800, "0001-01-01T00:00:00Z"),
            (-62_167_219_200, "0000-01-01T00:00:00Z"),
            (-62_198_755_200, "-0001-01-01T00:00:00Z"),
        ];

        for (seconds, expected) in cases {
            assert_eq!(
                format_unix_timestamp(seconds, 0).as_ref(),
                expected.as_bytes()
            );
        }
    }

    #[test]
    fn formats_extreme_duration_seconds() {
        let cases = [
            (
                i128::from(u64::MAX),
                "+584554051223-11-09T07:00:15.999999999Z",
            ),
            (
                -i128::from(u64::MAX) - 1,
                "-584554047284-02-23T16:59:44.999999999Z",
            ),
        ];

        for (seconds, expected) in cases {
            assert_eq!(
                format_unix_timestamp(seconds, 999_999_999).as_ref(),
                expected.as_bytes()
            );
        }
    }
}
