#[cfg(test)]
use crate::transformer::{CodeTransformer, RustAnalyzer};
#[cfg(test)]
use anyhow::Result;

#[cfg(test)]
/// Helper function to process a string of Rust code
pub fn process_code(code: &str, no_comments: bool, no_function_bodies: bool) -> Result<String> {
    use syn::visit_mut::VisitMut;

    let analyzer = RustAnalyzer::new(code)?;
    let mut transformer = CodeTransformer::new(no_comments, no_function_bodies);

    let mut ast = analyzer.ast;
    transformer.visit_file_mut(&mut ast);

    let output = prettyplease::unparse(&ast);

    Ok(output)
}
