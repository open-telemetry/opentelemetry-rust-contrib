#include "serialize_ffi.h"

// Suppress third-party warnings: -Wignored-qualifiers, -Wdeprecated-copy (GCC/Clang)
#ifdef __GNUC__
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wignored-qualifiers" // Ignore qualifiers on cast result (internal)
#pragma GCC diagnostic ignored "-Wdeprecated-copy"    // Ignore deprecated copy assignment (internal)
#endif

// Suppress third-party warnings on MSVC: 4267 (size_t to int), 4244 (type conversion), 4996 (deprecated)
#ifdef _MSC_VER
#pragma warning(push)
#pragma warning(disable: 4267) // Conversion from 'size_t' to 'type', possible loss of data (internal)
#pragma warning(disable: 4244) // Conversion from 'type1' to 'type2', possible loss of data (internal)
#pragma warning(disable: 4996) // Deprecated functions or unsafe functions (internal)
#endif

#include <bond/core/bond.h>
#include <bond/protocol/simple_binary.h>
#include <bond/core/schema.h>

#ifdef __GNUC__
#pragma GCC diagnostic pop
#endif

#ifdef _MSC_VER
#pragma warning(pop)
#endif
#include <cstring>
#include <vector>
#include <string>
#include <cstdlib>
#include <stdexcept>
#include <iostream>

namespace {

struct Field {
    std::string name;
    uint8_t type;
    uint16_t id;
};

std::vector<Field> parse_schema(const uint8_t* ptr, size_t len, size_t& out_count) {
    if (len < 2) throw std::runtime_error("schema too short");
    out_count = ptr[0] | (ptr[1] << 8);
    std::vector<Field> fields;
    ptr += 2;
    len -= 2;
    for (size_t i = 0; i < static_cast<size_t>(out_count); ++i){
        if (len < 1) throw std::runtime_error("not enough data for name_len");
        uint8_t name_len = *ptr++;
        len--;
        if (len < size_t(name_len) + 1 + 2) throw std::runtime_error("not enough data for field");
        std::string name(reinterpret_cast<const char*>(ptr), name_len);
        ptr += name_len;
        len -= name_len;
        uint8_t type = *ptr++;
        len--;
        uint16_t id = ptr[0] | (ptr[1] << 8);
        ptr += 2;
        len -= 2;
        fields.push_back({name, type, id});
    }
    return fields;
}

} // namespace

extern "C" SchemaResult* marshal_schema_ffi(const void* schema_buf, size_t schema_len, size_t* out_len) {
    try {
        const uint8_t* ptr = reinterpret_cast<const uint8_t*>(schema_buf);
        size_t field_count;
        auto fields = parse_schema(ptr, schema_len, field_count);

        bond::StructDef struct_def;
        struct_def.metadata.name = "MdsContainer";
        struct_def.metadata.qualified_name = "testNamespace.MdsContainer";
        struct_def.metadata.attributes = {};
        struct_def.metadata.modifier = bond::Modifier::Optional;
        
        for (const auto& f : fields) {
            bond::FieldDef fd;
            fd.id = f.id;
            fd.metadata.name = f.name;
            fd.type.bonded_type = false;
            fd.type.id = static_cast<bond::BondDataType>(f.type);
            struct_def.fields.push_back(std::move(fd));
        }
         // Allocate schemaDef on heap
         auto schemaDef = std::make_unique<bond::SchemaDef>();
         schemaDef->root.id = bond::BT_STRUCT;
         schemaDef->root.bonded_type = false;
         schemaDef->structs.push_back(struct_def);
 
         // Marshal to buffer
         bond::OutputBuffer buf;
         bond::SimpleBinaryWriter<bond::OutputBuffer> writer(buf);
         bond::Marshal(*schemaDef, writer);
         // Copy marshaled bytes
         auto marshaled = buf.GetBuffer();
         void* bytes = malloc(marshaled.size());        
         if (!bytes && marshaled.size()) throw std::bad_alloc();
         std::memcpy(bytes, marshaled.data(), marshaled.size());
 
         // Create SchemaResult and fill fields
         SchemaResult* result = new SchemaResult;
         result->schema_bytes = bytes;
         result->schema_bytes_len = marshaled.size();
         result->schema_ptr = schemaDef.release();
 
         *out_len = result->schema_bytes_len;
         return result;
     } catch (...) {
         *out_len = 0;
         return nullptr;
     }
 }

extern "C" void* marshal_row_ffi(void* schema_ptr,
                                      const void* row_buf, size_t row_len,
                                      size_t* out_len) {
    try {
        bond::SchemaDef* schemaDef = static_cast<bond::SchemaDef*>(schema_ptr);
        if (!schemaDef || schemaDef->structs.empty())
            throw std::runtime_error("invalid or empty schema");

        const auto& fields = schemaDef->structs[0].fields;
        const uint8_t* ptr = reinterpret_cast<const uint8_t*>(row_buf);
        size_t remain = row_len;

        bond::OutputBuffer buf;
        bond::SimpleBinaryWriter<bond::OutputBuffer> writer(buf);
        writer.WriteStructBegin(schemaDef->structs[0].metadata, false);

        for (const auto& f : fields) {
            switch (f.type.id) {
                case bond::BT_DOUBLE: {
                    if (remain < 8) throw std::runtime_error("row too short");
                    double v;
                    std::memcpy(&v, ptr, 8);
                    writer.Write(v);
                    ptr += 8; remain -= 8;
                    break;
                }
                case bond::BT_INT32: {
                    if (remain < 4) throw std::runtime_error("row too short");
                    int32_t v;
                    std::memcpy(&v, ptr, 4);
                    writer.Write(v);
                    ptr += 4; remain -= 4;
                    break;
                }
                case bond::BT_FLOAT: {
                    if (remain < 4) throw std::runtime_error("row too short for float");
                    float v;
                    std::memcpy(&v, ptr, 4);
                    writer.Write(v);
                    ptr += 4; remain -= 4;
                    break;
                }
                case bond::BT_STRING: {
                    if (remain < 4) throw std::runtime_error("row too short for string len");
                    uint32_t slen = ptr[0] | (ptr[1] << 8) | (ptr[2] << 16) | (ptr[3] << 24);
                    ptr += 4; remain -= 4;
                    if (remain < slen) throw std::runtime_error("row too short for string bytes");
                    std::string s(reinterpret_cast<const char*>(ptr), slen);
                    writer.Write(s);
                    ptr += slen; remain -= slen;
                    break;
                }
                case bond::BT_WSTRING: {
                    if (remain < 2) throw std::runtime_error("row too short for wstring len");
                    uint16_t slen = ptr[0] | (ptr[1] << 8);  // Read length in code units
                    ptr += 2; remain -= 2;
                    
                    // For UTF-16, each character is 2 bytes
                    size_t byte_len = slen * 2;
                    if (remain < byte_len) throw std::runtime_error("row too short for wstring bytes");
                    
                    // Create a u16string from the UTF-16LE encoded bytes
                    std::u16string ws;
                    ws.reserve(slen);
                    for (size_t i = 0; i < slen; i++) {
                        char16_t c = ptr[i*2] | (ptr[i*2+1] << 8);
                        ws.push_back(c);
                    }
                    
                    writer.Write(ws);
                    ptr += byte_len; remain -= byte_len;
                    break;
                }
                // Extend here for more Seializable types as needed
                default: {
                    throw std::runtime_error("unsupported type id");
                }
            }
        }
        writer.WriteStructEnd();
        // Copy the Serialized buffer directly to a malloc'd buffer
        auto output = buf.GetBuffer();
        *out_len = output.size();
        void* out = malloc(*out_len);
        if (!out && *out_len) throw std::bad_alloc();
        std::memcpy(out, output.data(), *out_len);
        return out;    
    } catch (...) {
        *out_len = 0;
        return nullptr;
    }
}

extern "C" void free_row_buf_ffi(void* ptr) {
    std::free(ptr);
}


extern "C" void free_schema_buf_ffi(SchemaResult* result) {
    if (result) {
        if (result->schema_bytes) free(result->schema_bytes);
        delete static_cast<bond::SchemaDef*>(result->schema_ptr);
        delete result;
    }
}