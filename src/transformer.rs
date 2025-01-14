use anyhow::{Context, Result};
use quote::ToTokens;
use syn::{
    parse_quote,
    visit_mut::{self, VisitMut},
    Attribute, File, GenericArgument, ImplItem, Item, ItemMod, ItemTrait, PathArguments,
    ReturnType, TraitItem, Type, TypePath,
};

pub struct RustAnalyzer {
    pub ast: File,
}

impl RustAnalyzer {
    /// Creates a new RustAnalyzer instance
    pub fn new(content: &str) -> Result<Self> {
        let ast = syn::parse_file(content)
            .with_context(|| "Failed to parse Rust file. Check for syntax errors")?;

        Ok(Self { ast })
    }

    /// Checks if a type is string-like, or a Result/Option containing a string-like type
    fn is_string_or_json_type(ty: &Type) -> bool {
        match ty {
            Type::Path(type_path) => {
                let path_str = type_path.path.to_token_stream().to_string();

                // Check for string-like types
                if path_str.contains("String")
                    || path_str.contains("str")
                    || path_str.contains("Cow<str>")
                {
                    return true;
                }

                // Check for Result/Option with string-like types
                if path_str.starts_with("Result") || path_str.starts_with("Option") {
                    if let Type::Path(TypePath { path, .. }) = ty {
                        if let Some(last_segment) = path.segments.last() {
                            if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
                                // Check the first type parameter of Result/Option
                                return args.args.iter().next().is_some_and(|arg| {
                                    if let GenericArgument::Type(inner_ty) = arg {
                                        Self::is_string_or_json_type(inner_ty)
                                    } else {
                                        false
                                    }
                                });
                            }
                        }
                    }
                }

                false
            }
            Type::Reference(type_ref) => Self::is_string_or_json_type(&type_ref.elem),
            _ => false,
        }
    }
}

pub struct CodeTransformer {
    no_comments: bool,
}

impl CodeTransformer {
    /// Creates a new CodeTransformer instance
    pub fn new(no_comments: bool) -> Self {
        Self { no_comments }
    }

    /// Gets attributes from any Item type
    fn get_attrs(item: &Item) -> &[Attribute] {
        match item {
            Item::Fn(f) => &f.attrs,
            Item::Mod(m) => &m.attrs,
            Item::Struct(s) => &s.attrs,
            Item::Enum(e) => &e.attrs,
            Item::Trait(t) => &t.attrs,
            Item::Impl(i) => &i.attrs,
            Item::Type(t) => &t.attrs,
            Item::Const(c) => &c.attrs,
            Item::Static(s) => &s.attrs,
            Item::Use(u) => &u.attrs,
            Item::ExternCrate(e) => &e.attrs,
            Item::ForeignMod(f) => &f.attrs,
            Item::Macro(m) => &m.attrs,
            Item::TraitAlias(t) => &t.attrs,
            Item::Union(u) => &u.attrs,
            _ => &[],
        }
    }

    /// Gets mutable attributes from any Item type
    fn get_attrs_mut(item: &mut Item) -> &mut Vec<Attribute> {
        match item {
            Item::Fn(f) => &mut f.attrs,
            Item::Mod(m) => &mut m.attrs,
            Item::Struct(s) => &mut s.attrs,
            Item::Enum(e) => &mut e.attrs,
            Item::Trait(t) => &mut t.attrs,
            Item::Impl(i) => &mut i.attrs,
            Item::Type(t) => &mut t.attrs,
            Item::Const(c) => &mut c.attrs,
            Item::Static(s) => &mut s.attrs,
            Item::Use(u) => &mut u.attrs,
            Item::ExternCrate(e) => &mut e.attrs,
            Item::ForeignMod(f) => &mut f.attrs,
            Item::Macro(m) => &mut m.attrs,
            Item::TraitAlias(t) => &mut t.attrs,
            Item::Union(u) => &mut u.attrs,
            _ => panic!("Unexpected item type"),
        }
    }

    /// Checks if an item has test-related attributes
    fn has_test_attribute(attrs: &[Attribute]) -> bool {
        attrs.iter().any(|attr| {
            attr.path().is_ident("test")
                || (attr.path().is_ident("cfg") && Self::is_cfg_test_attribute(attr))
        })
    }

    /// Checks if an attribute is #[cfg(test)]
    fn is_cfg_test_attribute(attr: &Attribute) -> bool {
        if !attr.path().is_ident("cfg") {
            return false;
        }

        match attr.meta {
            syn::Meta::List(ref list) => list.tokens.to_string().contains("test"),
            _ => false,
        }
    }

    fn should_remove_item(item: &Item) -> bool {
        let attrs = Self::get_attrs(item);
        attrs.iter().any(|attr| {
            attr.path().is_ident("test") || 
            matches!(attr.meta, syn::Meta::List(ref list) if list.path.is_ident("cfg") && list.tokens.to_string().contains("test"))
        })
    }

    /// Checks if an implementation block is derived
    fn is_derived_implementation(impl_block: &syn::ItemImpl) -> bool {
        Self::get_attrs(&Item::Impl(impl_block.clone()))
            .iter()
            .any(|attr| attr.path().is_ident("derive"))
    }

    /// Checks if an implementation block is for the Serialize trait
    fn is_serialize_impl(impl_block: &syn::ItemImpl) -> bool {
        if let Some((_, trait_path, _)) = &impl_block.trait_ {
            let trait_str = quote::quote!(#trait_path).to_string();
            trait_str.contains("Serialize")
        } else {
            false
        }
    }

    /// Determines whether a method's body should be preserved
    /// Analyzes return type to determine if it's string-like
    fn analyze_return_type(ret_type: &ReturnType) -> bool {
        match ret_type {
            ReturnType::Default => false,
            ReturnType::Type(_, ty) => RustAnalyzer::is_string_or_json_type(ty),
        }
    }

    /// Processes attributes based on comment removal flag
    fn process_attributes(attrs: &mut Vec<Attribute>, no_comments: bool) {
        if no_comments {
            attrs.retain(|attr| !attr.path().is_ident("doc"));
        }
    }

    /// Adds appropriate comments for trait methods
    fn add_trait_method_comment(trait_item: &mut TraitItem, no_comments: bool) {
        if let TraitItem::Fn(method) = trait_item {
            if no_comments {
                // If no_comments is true, remove all doc comments
                method.attrs.retain(|attr| !attr.path().is_ident("doc"));
                return;
            }

            // First collect all existing doc comments
            let doc_comments = method
                .attrs
                .iter()
                .filter_map(|attr| {
                    if attr.path().is_ident("doc") {
                        if let Ok(meta) = attr.meta.require_name_value() {
                            if let syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(s),
                                ..
                            }) = &meta.value
                            {
                                return Some(s.value());
                            }
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();

            // Clear existing doc attributes
            method.attrs.retain(|attr| !attr.path().is_ident("doc"));

            // Prepare all new attributes at once
            let mut new_attrs = Vec::new();

            // Add the required/default implementation comment first
            let status_comment = if method.default.is_none() {
                parse_quote!(#[doc = " This is a required method"])
            } else {
                parse_quote!(#[doc = " There is a default implementation"])
            };
            new_attrs.push(status_comment);

            // Add an empty doc line if there are existing comments
            if !doc_comments.is_empty() {
                new_attrs.push(parse_quote!(#[doc = ""]));
            }

            // Add back the existing doc comments
            for comment in doc_comments {
                let doc_attr: syn::Attribute = parse_quote!(#[doc = #comment]);
                new_attrs.push(doc_attr);
            }

            // Extend the attributes with all new ones at once
            method.attrs.extend(new_attrs);
        }
    }
}

impl VisitMut for CodeTransformer {
    fn visit_item_mod_mut(&mut self, node: &mut ItemMod) {
        // Process module attributes
        Self::process_attributes(&mut node.attrs, self.no_comments);

        // Process inner items if they exist
        if let Some((_, items)) = &mut node.content {
            // Visit each item in the module
            for item in items.iter_mut() {
                self.visit_item_mut(item);
            }
        }
    }

    fn visit_item_trait_mut(&mut self, node: &mut ItemTrait) {
        // Process trait-level comments if needed
        if self.no_comments {
            node.attrs.retain(|attr| !attr.path().is_ident("doc"));
        }

        // Process trait items
        for item in &mut node.items {
            if let TraitItem::Fn(method) = item {
                // Process method comments if needed
                if self.no_comments {
                    method.attrs.retain(|attr| !attr.path().is_ident("doc"));
                }

                // Clear default implementation bodies
                if method.default.is_some() {
                    method.default = Some(parse_quote!({}));
                }
            }
        }

        visit_mut::visit_item_trait_mut(self, node);
    }

    /// Visits a file and removes test-related items
    fn visit_file_mut(&mut self, file: &mut syn::File) {
        // Process file-level attributes if no_comments is true
        if self.no_comments {
            file.attrs.retain(|attr| !attr.path().is_ident("doc"));
        }

        // Remove all test-related items
        file.items.retain(|item| !Self::should_remove_item(item));

        // Process remaining items
        for item in &mut file.items {
            self.visit_item_mut(item);
        }
    }

    fn visit_item_mut(&mut self, item: &mut Item) {
        // Skip test-related items
        if Self::has_test_attribute(Self::get_attrs(item)) {
            return;
        }

        match item {
            Item::Mod(item_mod) => {
                if Self::has_test_attribute(&item_mod.attrs) {
                    if let Some((_, items)) = &mut item_mod.content {
                        items.clear();
                    }
                    return;
                }

                // Process module attributes
                Self::process_attributes(&mut item_mod.attrs, self.no_comments);

                if let Some((_, items)) = &mut item_mod.content {
                    // Remove test items from the module
                    items.retain(|item| !Self::has_test_attribute(Self::get_attrs(item)));

                    // Process remaining items
                    for item in items {
                        // Process attributes before visiting the item
                        Self::process_attributes(Self::get_attrs_mut(item), self.no_comments);
                        self.visit_item_mut(item);
                    }
                }
            }
            Item::Fn(item_fn) => {
                // Process function-level comments
                Self::process_attributes(&mut item_fn.attrs, self.no_comments);

                // Replace with empty block
                item_fn.block = parse_quote!({});
            }
            Item::Trait(item_trait) => {
                // Process trait-level comments
                Self::process_attributes(&mut item_trait.attrs, self.no_comments);

                // Process trait methods
                for trait_item in &mut item_trait.items {
                    if let TraitItem::Fn(method) = trait_item {
                        // First process the attributes
                        Self::process_attributes(&mut method.attrs, self.no_comments);

                        // Then handle the default implementation
                        if method.default.is_some()
                            && !Self::analyze_return_type(&method.sig.output)
                        {
                            method.default = Some(parse_quote!({}));
                        }
                    }

                    // Finally add the trait method comment
                    Self::add_trait_method_comment(trait_item, self.no_comments);
                }
            }
            Item::Impl(item_impl) => {
                // Process impl block comments
                Self::process_attributes(&mut item_impl.attrs, self.no_comments);

                // Check implementation type before processing methods
                let is_derived = Self::is_derived_implementation(item_impl);
                let is_serialize = Self::is_serialize_impl(item_impl);

                // Process implementation methods
                for impl_item in &mut item_impl.items {
                    if let ImplItem::Fn(method) = impl_item {
                        Self::process_attributes(&mut method.attrs, self.no_comments);

                        if is_derived
                            || (!is_serialize && !Self::analyze_return_type(&method.sig.output))
                        {
                            method.block = parse_quote!({});
                        }
                    }
                }
            }
            Item::Struct(item_struct) => {
                // Process struct-level comments
                Self::process_attributes(&mut item_struct.attrs, self.no_comments);

                // Process field-level comments
                for field in &mut item_struct.fields {
                    Self::process_attributes(&mut field.attrs, self.no_comments);
                }
                visit_mut::visit_item_struct_mut(self, item_struct);
            }
            Item::Enum(item_enum) => {
                // Process enum-level comments
                Self::process_attributes(&mut item_enum.attrs, self.no_comments);
                visit_mut::visit_item_enum_mut(self, item_enum);
            }
            _ => visit_mut::visit_item_mut(self, item),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use syn::visit_mut::VisitMut;

    /// Helper function to process a string of Rust code
    fn process_code(code: &str, no_comments: bool) -> Result<String> {
        let analyzer = RustAnalyzer::new(code)?;
        let mut transformer = CodeTransformer::new(no_comments);

        let mut ast = analyzer.ast;
        transformer.visit_file_mut(&mut ast);

        let output = prettyplease::unparse(&ast);

        Ok(output)
    }

    #[test]
    fn test_regular_function() -> Result<()> {
        let input = r#"
            fn add(a: i32, b: i32) -> i32 {
                a + b
            }
        "#;

        let result = process_code(input, false)?;

        let expected = r#"fn add(a: i32, b: i32) -> i32 {}"#;

        assert_eq!(result.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_string_returning_function() -> Result<()> {
        let input = r#"
        impl MyStruct {
            fn to_string(&self) -> String {
                "test".to_string()
            }
        }
    "#;
        let expected = r#"impl MyStruct {
    fn to_string(&self) -> String {
        "test".to_string()
    }
}"#;
        assert_eq!(process_code(input, false)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_derived_serialize() -> Result<()> {
        let input = r#"
        #[derive(Serialize)]
        struct MyStruct {
            field: String,
        }
        
        impl MyStruct {
            fn serialize(&self) -> String {
                serde_json::to_string(self).unwrap()
            }
        }
    "#;
        let expected = r#"#[derive(Serialize)]
struct MyStruct {
    field: String,
}
impl MyStruct {
    fn serialize(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}"#;
        assert_eq!(process_code(input, false)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_option_string_function() -> Result<()> {
        let input = r#"
        impl MyStruct {
            fn get_optional_string(&self) -> Option<String> {
                Some("test".to_string())
            }
            
            fn get_optional_str(&self) -> Option<&str> {
                Some("test")
            }
            
            fn get_number(&self) -> Option<i32> {
                Some(42)
            }
        }
    "#;
        let expected = r#"impl MyStruct {
    fn get_optional_string(&self) -> Option<String> {
        Some("test".to_string())
    }
    fn get_optional_str(&self) -> Option<&str> {
        Some("test")
    }
    fn get_number(&self) -> Option<i32> {}
}"#;
        assert_eq!(process_code(input, false)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_custom_serialize() -> Result<()> {
        let input = r#"
        impl Serialize for MyStruct {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let mut state = serializer.serialize_struct("MyStruct", 1)?;
                state.serialize_field("field", &self.field)?;
                state.end()
            }
        }
    "#;
        let expected = r#"impl Serialize for MyStruct {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MyStruct", 1)?;
        state.serialize_field("field", &self.field)?;
        state.end()
    }
}"#;
        assert_eq!(process_code(input, false)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_trait_methods() -> Result<()> {
        // Test cases with comments enabled
        let test_cases = vec![
            (
                // Case 1: Required method with existing doc comment
                r#"trait MyTrait {
    /// Existing doc comment
    fn required_method(&self) -> i32;
}"#,
                r#"trait MyTrait {
    /// This is a required method
    ///
    /// Existing doc comment
    fn required_method(&self) -> i32;
}"#,
            ),
            (
                // Case 2: Default method with existing doc comment
                r#"trait MyTrait {
    /// Existing doc comment
    fn default_method(&self) -> i32 {
        42
    }
}"#,
                r#"trait MyTrait {
    /// There is a default implementation
    ///
    /// Existing doc comment
    fn default_method(&self) -> i32 {}
}"#,
            ),
            (
                // Case 3: Multiple methods with mixed comments
                r#"trait MyTrait {
    /// First required method
    fn required1(&self) -> i32;

    /// Default implementation
    fn default1(&self) -> i32 {
        42
    }

    fn required2(&self) -> i32;

    fn default2(&self) -> i32 {
        43
    }
}"#,
                r#"trait MyTrait {
    /// This is a required method
    ///
    /// First required method
    fn required1(&self) -> i32;
    /// There is a default implementation
    ///
    /// Default implementation
    fn default1(&self) -> i32 {}
    /// This is a required method
    fn required2(&self) -> i32;
    /// There is a default implementation
    fn default2(&self) -> i32 {}
}"#,
            ),
        ];

        // Test with comments enabled
        for (input, expected) in test_cases {
            assert_eq!(
                process_code(input, false)?.trim(),
                expected.trim(),
                "Failed with comments enabled"
            );
        }

        // Test with comments disabled
        let no_comments_input = r#"trait MyTrait {
    /// Should be removed
    fn required_method(&self) -> i32;

    /// Should also be removed
    fn default_method(&self) -> i32 {
        42
    }
}"#;

        let no_comments_expected = r#"trait MyTrait {
    fn required_method(&self) -> i32;
    fn default_method(&self) -> i32 {}
}"#;

        assert_eq!(
            process_code(no_comments_input, true)?.trim(),
            no_comments_expected.trim(),
            "Failed with comments disabled"
        );

        Ok(())
    }

    #[test]
    fn test_line_doc_comments() -> Result<()> {
        let input = r#"
        //! A doc comment that applies to the module
        //!! Still a module doc comment
        
        /// Outer line doc
        //// Just a regular comment
        //   Just a regular comment
        struct MyStruct {
            /// Field documentation
            field: String,
        }
        "#;

        let expected_with_comments = r#"//! A doc comment that applies to the module
//!! Still a module doc comment
/// Outer line doc
struct MyStruct {
    /// Field documentation
    field: String,
}"#;

        let expected_no_comments = r#"struct MyStruct {
    field: String,
}"#;

        assert_eq!(
            process_code(input, false)?.trim(),
            expected_with_comments.trim()
        );
        assert_eq!(
            process_code(input, true)?.trim(),
            expected_no_comments.trim()
        );
        Ok(())
    }

    #[test]
    fn test_block_doc_comments() -> Result<()> {
        let input = r#"
        /*! Inner block doc */
        /*!! Still an inner block doc */
        
        /** Outer block doc */
        /*** Just a regular comment */
        /* Just a regular comment */
        struct MyStruct {
            /** Field block documentation */
            field: String,
        }
        "#;

        let expected_with_comments = r#"//! Inner block doc
//!! Still an inner block doc
/// Outer block doc
struct MyStruct {
    /// Field block documentation
    field: String,
}"#;

        let expected_no_comments = r#"struct MyStruct {
    field: String,
}"#;

        assert_eq!(
            process_code(input, false)?.trim(),
            expected_with_comments.trim()
        );
        assert_eq!(
            process_code(input, true)?.trim(),
            expected_no_comments.trim()
        );
        Ok(())
    }

    #[test]
    fn test_nested_comments() -> Result<()> {
        let input = r#"
        /* In Rust /* we can /* nest comments */ */ */
        
        /* Contains doc comments:
           /*! inner block */ 
           /** outer block */
           /// line doc
        */
        
        /** Outer block containing /* nested */ comments */
        struct MyStruct {
            field: String,
        }
        "#;

        let expected_with_comments = r#"/// Outer block containing /* nested */ comments
struct MyStruct {
    field: String,
}"#;

        let expected_no_comments = r#"struct MyStruct {
    field: String,
}"#;

        assert_eq!(
            process_code(input, false)?.trim(),
            expected_with_comments.trim()
        );
        assert_eq!(
            process_code(input, true)?.trim(),
            expected_no_comments.trim()
        );
        Ok(())
    }

    #[test]
    fn test_degenerate_comment_cases() -> Result<()> {
        let input = r#"
        #[doc = "Empty module doc"]
        #[doc = "Empty block doc"]
        // Regular comment
        #[doc = "Empty outer doc"]
        /* Regular block */
        #[doc = "Empty outer block"]
        struct MyStruct {
            #[doc = "Empty inner doc"]
            #[doc = "Empty inner block"]
            field: String,
        }
        "#;

        let expected_with_comments = r#"///Empty module doc
///Empty block doc
///Empty outer doc
///Empty outer block
struct MyStruct {
    ///Empty inner doc
    ///Empty inner block
    field: String,
}"#;

        let expected_no_comments = r#"struct MyStruct {
    field: String,
}"#;

        assert_eq!(
            process_code(input, false)?.trim(),
            expected_with_comments.trim()
        );
        assert_eq!(
            process_code(input, true)?.trim(),
            expected_no_comments.trim()
        );
        Ok(())
    }

    #[test]
    fn test_mixed_comments_in_module() -> Result<()> {
        let input = r#"
        //! Module documentation
        pub mod outer_module {
            //! Inner line doc
            /*! Inner block doc */
            
            /// Outer line doc
            /** Outer block doc */
            
            // Regular comment
            /* Regular block comment */
            
            pub struct MyStruct {
                /// Field doc
                /** Block field doc */
                field: String,
            }
            
            /// Function doc
            /** Block function doc */
            fn my_func() -> i32 {
                42
            }
        }
        "#;

        let expected_with_comments = r#"//! Module documentation
pub mod outer_module {
    //! Inner line doc
    //! Inner block doc
    /// Outer line doc
    /// Outer block doc
    pub struct MyStruct {
        /// Field doc
        /// Block field doc
        field: String,
    }
    /// Function doc
    /// Block function doc
    fn my_func() -> i32 {}
}"#;

        let expected_no_comments = r#"pub mod outer_module {
    pub struct MyStruct {
        field: String,
    }
    fn my_func() -> i32 {}
}"#;

        assert_eq!(
            process_code(input, false)?.trim(),
            expected_with_comments.trim()
        );
        assert_eq!(
            process_code(input, true)?.trim(),
            expected_no_comments.trim()
        );
        Ok(())
    }
}
