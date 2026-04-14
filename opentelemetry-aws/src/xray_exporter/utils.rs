/// Generates setter methods for builder pattern fields.
///
/// Supports both by-reference (`&mut self`) and by-value (`self`) variants.
#[macro_export]
macro_rules! field_setter {
    (no_ref $field_name:ident : $field_type:ty) => {
        #[doc = "Sets the corresponding field of the builder."]
        pub fn $field_name(mut self, $field_name:$field_type) -> Self {
            self.$field_name = Some($field_name);
            self
        }
    };
    ($field_name:ident : $field_type:ty) => {
        #[doc = "Sets the corresponding field of the builder."]
        pub fn $field_name(&mut self, $field_name:$field_type) -> &mut Self {
            self.$field_name = Some($field_name);
            self
        }
    };
    (no_ref $field_name:ident) => {
        field_setter!(no_ref $field_name : Cow<'a, str>);
    };
    ($field_name:ident) => {
        field_setter!($field_name : Cow<'a, str>);
    };
}

/// Generates setter methods for boolean flag fields.
///
/// Sets the flag to `true` when called, supporting both by-reference and by-value variants.
#[macro_export]
macro_rules! flag_setter {
    ($field_name:ident) => {
        #[doc = "Sets the corresponding flag of the builder to `true`."]
        pub fn $field_name(&mut self) -> &mut Self {
            self.$field_name = true;
            self
        }
    };
    (no_ref $field_name:ident) => {
        #[doc = "Sets the corresponding flag of the builder to `true`."]
        pub fn $field_name(mut self) -> Self {
            self.$field_name = true;
            self
        }
    };
}
