use quote::ToTokens;
use std::io::{self, Read};
use syn::{Item, ItemFn};

fn eligible_test(function: &ItemFn) -> bool {
    let mut test = false;
    for attr in &function.attrs {
        let path = attr.path();
        if path.is_ident("test")
            || path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .eq(["tokio", "test"].map(String::from))
        {
            test = true;
        } else if !path.is_ident("doc") && !path.is_ident("should_panic") {
            return false;
        }
    }
    test
}

// A changed helper, attribute, declaration or module may affect other tests.
// Only erase the bodies of existing, independently registered test functions.
fn changed_tests(before: &str, after: &str) -> Option<Vec<String>> {
    let before = syn::parse_file(before).ok()?;
    let after = syn::parse_file(after).ok()?;
    if before.items.len() != after.items.len()
        || before
            .attrs
            .iter()
            .map(ToTokens::to_token_stream)
            .map(|t| t.to_string())
            .ne(after
                .attrs
                .iter()
                .map(ToTokens::to_token_stream)
                .map(|t| t.to_string()))
    {
        return None;
    }

    let mut names = Vec::new();
    for (old, new) in before.items.into_iter().zip(after.items) {
        if old.to_token_stream().to_string() == new.to_token_stream().to_string() {
            continue;
        }
        let (Item::Fn(mut old), Item::Fn(mut new)) = (old, new) else {
            return None;
        };
        if !eligible_test(&old) || !eligible_test(&new) {
            return None;
        }
        old.block.stmts.clear();
        new.block.stmts.clear();
        if old.to_token_stream().to_string() != new.to_token_stream().to_string() {
            return None;
        }
        names.push(new.sig.ident.to_string());
    }
    Some(names)
}

fn main() -> io::Result<()> {
    let mut source = String::new();
    io::stdin().read_to_string(&mut source)?;
    if let Some((before, after)) = source.split_once('\0')
        && let Some(tests) = changed_tests(before, after)
    {
        for test in tests {
            println!("{test}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::changed_tests;

    #[test]
    fn focuses_changed_test_bodies() {
        let old = "#[tokio::test] async fn first() { old().await; } #[test] fn second() {}";
        let new = "#[tokio::test] async fn first() { new().await; } #[test] fn second() {}";
        assert_eq!(changed_tests(old, new), Some(vec!["first".into()]));
    }

    #[test]
    fn shared_code_and_test_metadata_require_the_target() {
        let original = "fn helper() {} #[test] fn first() { helper(); }";
        for modified in [
            "fn helper() { panic!(); } #[test] fn first() { helper(); }",
            "fn helper() {} #[test] #[ignore] fn first() { helper(); }",
            "fn helper() {} #[test] fn renamed() { helper(); }",
            "fn helper() {}",
        ] {
            assert_eq!(changed_tests(original, modified), None);
        }
    }
}
