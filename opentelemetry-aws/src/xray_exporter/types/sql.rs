use std::borrow::Cow;

use serde::Serialize;

use crate::field_setter;

use super::{
    error::Result,
    utils::{verify_string_length, MaybeSkip},
};

/// SQL query information for database operations.
///
/// Records information about SQL queries that your application makes to databases.
#[derive(Debug, Serialize)]
pub(super) struct SqlData<'a> {
    /// Database connection string, excluding passwords
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    connection_string: Option<Cow<'a, str>>,

    /// Database URL connection string, excluding passwords
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    url: Option<Cow<'a, str>>,

    /// The database query with user-provided values removed or replaced by placeholders
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    sanitized_query: Option<Cow<'a, str>>,

    /// The name of the database engine
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    database_type: Option<Cow<'a, str>>,

    /// The database username (limited to 250 characters)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    user: Option<Cow<'a, str>>,
}

impl MaybeSkip for SqlData<'_> {
    /// Returns true if this SQL data is empty (all fields are None)
    fn skip(&self) -> bool {
        self.connection_string.skip()
            && self.url.skip()
            && self.sanitized_query.skip()
            && self.database_type.skip()
            && self.user.skip()
    }
}

/// Builder for constructing SQL query metadata.
#[derive(Debug, Default)]
pub(crate) struct SqlDataBuilder<'a> {
    connection_string: Option<Cow<'a, str>>,
    url: Option<Cow<'a, str>>,
    sanitized_query: Option<Cow<'a, str>>,
    database_type: Option<Cow<'a, str>>,
    user: Option<Cow<'a, str>>,
}

impl<'a> SqlDataBuilder<'a> {
    field_setter!(connection_string);
    field_setter!(url);
    field_setter!(sanitized_query);
    field_setter!(database_type);

    /// Sets the database user.
    ///
    /// # Arguments
    ///
    /// * `user` - The database username
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::StringTooLong(250)` if the user is longer than 250 characters.
    pub fn user(&mut self, user: Cow<'a, str>) -> Result<&mut Self> {
        verify_string_length(user.as_ref(), 250)?;
        self.user = Some(user);
        Ok(self)
    }

    /// Builds the `SqlData` instance.
    pub(super) fn build(self) -> SqlData<'a> {
        SqlData {
            connection_string: self.connection_string,
            url: self.url,
            sanitized_query: self.sanitized_query,
            database_type: self.database_type,
            user: self.user,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::error::ConstraintError;

    #[test]
    fn sql_data_builder_user_valid() {
        // Valid user with exactly 250 characters
        let user_250 = "a".repeat(250);
        let mut builder = SqlDataBuilder::default();
        let result = builder.user(Cow::Owned(user_250.clone()));
        assert!(result.is_ok());
        assert_eq!(builder.user.as_ref().unwrap().as_ref(), user_250);

        // Valid user with less than 250 characters
        let mut builder = SqlDataBuilder::default();
        let result = builder.user(Cow::Borrowed("db_user"));
        assert!(result.is_ok());
        assert_eq!(builder.user.as_ref().unwrap().as_ref(), "db_user");

        // Valid user with 1 character
        let mut builder = SqlDataBuilder::default();
        let result = builder.user(Cow::Borrowed("u"));
        assert!(result.is_ok());
        assert_eq!(builder.user.as_ref().unwrap().as_ref(), "u");
    }

    #[test]
    fn sql_data_builder_user_invalid() {
        // Invalid user with 251 characters (exceeds limit)
        let user_251 = "a".repeat(251);
        let mut builder = SqlDataBuilder::default();
        let result = builder.user(Cow::Owned(user_251));
        assert!(matches!(result, Err(ConstraintError::StringTooLong(250))));

        // Invalid user with significantly more than 250 characters
        let user_1000 = "x".repeat(1000);
        let mut builder = SqlDataBuilder::default();
        let result = builder.user(Cow::Owned(user_1000));
        assert!(matches!(result, Err(ConstraintError::StringTooLong(250))));
    }
}
