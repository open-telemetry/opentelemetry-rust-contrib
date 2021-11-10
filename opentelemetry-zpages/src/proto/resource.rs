// This file is generated by rust-protobuf 2.25.2. Do not edit
// @generated

// https://github.com/rust-lang/rust-clippy/issues/702
#![allow(unknown_lints)]
#![allow(clippy::all)]

#![allow(unused_attributes)]
#![cfg_attr(rustfmt, rustfmt::skip)]

#![allow(box_pointers)]
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(trivial_casts)]
#![allow(unused_imports)]
#![allow(unused_results)]
//! Generated file from `opentelemetry/proto/resource/v1/resource.proto`

/// Generated files are compatible only with the same version
/// of protobuf runtime.
// const _PROTOBUF_VERSION_CHECK: () = ::protobuf::VERSION_2_25_2;

#[derive(PartialEq,Clone,Default)]
#[cfg_attr(feature = "with-serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub struct Resource {
    // message fields
    pub attributes: ::protobuf::RepeatedField<super::common::KeyValue>,
    pub dropped_attributes_count: u32,
    // special fields
    #[cfg_attr(feature = "with-serde", serde(skip))]
    pub unknown_fields: ::protobuf::UnknownFields,
    #[cfg_attr(feature = "with-serde", serde(skip))]
    pub cached_size: ::protobuf::CachedSize,
}

impl<'a> ::std::default::Default for &'a Resource {
    fn default() -> &'a Resource {
        <Resource as ::protobuf::Message>::default_instance()
    }
}

impl Resource {
    pub fn new() -> Resource {
        ::std::default::Default::default()
    }

    // repeated .opentelemetry.proto.common.v1.KeyValue attributes = 1;


    pub fn get_attributes(&self) -> &[super::common::KeyValue] {
        &self.attributes
    }
    pub fn clear_attributes(&mut self) {
        self.attributes.clear();
    }

    // Param is passed by value, moved
    pub fn set_attributes(&mut self, v: ::protobuf::RepeatedField<super::common::KeyValue>) {
        self.attributes = v;
    }

    // Mutable pointer to the field.
    pub fn mut_attributes(&mut self) -> &mut ::protobuf::RepeatedField<super::common::KeyValue> {
        &mut self.attributes
    }

    // Take field
    pub fn take_attributes(&mut self) -> ::protobuf::RepeatedField<super::common::KeyValue> {
        ::std::mem::replace(&mut self.attributes, ::protobuf::RepeatedField::new())
    }

    // uint32 dropped_attributes_count = 2;


    pub fn get_dropped_attributes_count(&self) -> u32 {
        self.dropped_attributes_count
    }
    pub fn clear_dropped_attributes_count(&mut self) {
        self.dropped_attributes_count = 0;
    }

    // Param is passed by value, moved
    pub fn set_dropped_attributes_count(&mut self, v: u32) {
        self.dropped_attributes_count = v;
    }
}

impl ::protobuf::Message for Resource {
    fn is_initialized(&self) -> bool {
        for v in &self.attributes {
            if !v.is_initialized() {
                return false;
            }
        };
        true
    }

    fn merge_from(&mut self, is: &mut ::protobuf::CodedInputStream<'_>) -> ::protobuf::ProtobufResult<()> {
        while !is.eof()? {
            let (field_number, wire_type) = is.read_tag_unpack()?;
            match field_number {
                1 => {
                    ::protobuf::rt::read_repeated_message_into(wire_type, is, &mut self.attributes)?;
                },
                2 => {
                    if wire_type != ::protobuf::wire_format::WireTypeVarint {
                        return ::std::result::Result::Err(::protobuf::rt::unexpected_wire_type(wire_type));
                    }
                    let tmp = is.read_uint32()?;
                    self.dropped_attributes_count = tmp;
                },
                _ => {
                    ::protobuf::rt::read_unknown_or_skip_group(field_number, wire_type, is, self.mut_unknown_fields())?;
                },
            };
        }
        ::std::result::Result::Ok(())
    }

    // Compute sizes of nested messages
    #[allow(unused_variables)]
    fn compute_size(&self) -> u32 {
        let mut my_size = 0;
        for value in &self.attributes {
            let len = value.compute_size();
            my_size += 1 + ::protobuf::rt::compute_raw_varint32_size(len) + len;
        };
        if self.dropped_attributes_count != 0 {
            my_size += ::protobuf::rt::value_size(2, self.dropped_attributes_count, ::protobuf::wire_format::WireTypeVarint);
        }
        my_size += ::protobuf::rt::unknown_fields_size(self.get_unknown_fields());
        self.cached_size.set(my_size);
        my_size
    }

    fn write_to_with_cached_sizes(&self, os: &mut ::protobuf::CodedOutputStream<'_>) -> ::protobuf::ProtobufResult<()> {
        for v in &self.attributes {
            os.write_tag(1, ::protobuf::wire_format::WireTypeLengthDelimited)?;
            os.write_raw_varint32(v.get_cached_size())?;
            v.write_to_with_cached_sizes(os)?;
        };
        if self.dropped_attributes_count != 0 {
            os.write_uint32(2, self.dropped_attributes_count)?;
        }
        os.write_unknown_fields(self.get_unknown_fields())?;
        ::std::result::Result::Ok(())
    }

    fn get_cached_size(&self) -> u32 {
        self.cached_size.get()
    }

    fn get_unknown_fields(&self) -> &::protobuf::UnknownFields {
        &self.unknown_fields
    }

    fn mut_unknown_fields(&mut self) -> &mut ::protobuf::UnknownFields {
        &mut self.unknown_fields
    }

    fn as_any(&self) -> &dyn (::std::any::Any) {
        self as &dyn (::std::any::Any)
    }
    fn as_any_mut(&mut self) -> &mut dyn (::std::any::Any) {
        self as &mut dyn (::std::any::Any)
    }
    fn into_any(self: ::std::boxed::Box<Self>) -> ::std::boxed::Box<dyn (::std::any::Any)> {
        self
    }

    fn descriptor(&self) -> &'static ::protobuf::reflect::MessageDescriptor {
        Self::descriptor_static()
    }

    fn new() -> Resource {
        Resource::new()
    }

    fn descriptor_static() -> &'static ::protobuf::reflect::MessageDescriptor {
        static descriptor: ::protobuf::rt::LazyV2<::protobuf::reflect::MessageDescriptor> = ::protobuf::rt::LazyV2::INIT;
        descriptor.get(|| {
            let mut fields = ::std::vec::Vec::new();
            fields.push(::protobuf::reflect::accessor::make_repeated_field_accessor::<_, ::protobuf::types::ProtobufTypeMessage<super::common::KeyValue>>(
                "attributes",
                |m: &Resource| { &m.attributes },
                |m: &mut Resource| { &mut m.attributes },
            ));
            fields.push(::protobuf::reflect::accessor::make_simple_field_accessor::<_, ::protobuf::types::ProtobufTypeUint32>(
                "dropped_attributes_count",
                |m: &Resource| { &m.dropped_attributes_count },
                |m: &mut Resource| { &mut m.dropped_attributes_count },
            ));
            ::protobuf::reflect::MessageDescriptor::new_pb_name::<Resource>(
                "Resource",
                fields,
                file_descriptor_proto()
            )
        })
    }

    fn default_instance() -> &'static Resource {
        static instance: ::protobuf::rt::LazyV2<Resource> = ::protobuf::rt::LazyV2::INIT;
        instance.get(Resource::new)
    }
}

impl ::protobuf::Clear for Resource {
    fn clear(&mut self) {
        self.attributes.clear();
        self.dropped_attributes_count = 0;
        self.unknown_fields.clear();
    }
}

impl ::std::fmt::Debug for Resource {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        ::protobuf::text_format::fmt(self, f)
    }
}

impl ::protobuf::reflect::ProtobufValue for Resource {
    fn as_ref(&self) -> ::protobuf::reflect::ReflectValueRef {
        ::protobuf::reflect::ReflectValueRef::Message(self)
    }
}

static file_descriptor_proto_data: &'static [u8] = b"\
    \n.opentelemetry/proto/resource/v1/resource.proto\x12\x1fopentelemetry.p\
    roto.resource.v1\x1a*opentelemetry/proto/common/v1/common.proto\"\x8d\
    \x01\n\x08Resource\x12G\n\nattributes\x18\x01\x20\x03(\x0b2'.opentelemet\
    ry.proto.common.v1.KeyValueR\nattributes\x128\n\x18dropped_attributes_co\
    unt\x18\x02\x20\x01(\rR\x16droppedAttributesCountBw\n\"io.opentelemetry.\
    proto.resource.v1B\rResourceProtoP\x01Z@github.com/open-telemetry/opente\
    lemetry-proto/gen/go/resource/v1b\x06proto3\
";

static file_descriptor_proto_lazy: ::protobuf::rt::LazyV2<::protobuf::descriptor::FileDescriptorProto> = ::protobuf::rt::LazyV2::INIT;

fn parse_descriptor_proto() -> ::protobuf::descriptor::FileDescriptorProto {
    ::protobuf::Message::parse_from_bytes(file_descriptor_proto_data).unwrap()
}

pub fn file_descriptor_proto() -> &'static ::protobuf::descriptor::FileDescriptorProto {
    file_descriptor_proto_lazy.get(|| {
        parse_descriptor_proto()
    })
}