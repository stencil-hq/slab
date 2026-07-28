//! Integration test: expand a real document and drive the generated `Doc`.

slab_macro::include_doc!(counter, "tests/fixtures/counter.slab");

#[test]
fn generated_module_drives_the_document() {
    assert_eq!(counter::PARAM_LABEL, 0);
    assert!(!counter::SLIR.is_empty());

    let mut doc = counter::Doc::new();
    assert!(doc.ok());
    assert!(doc.set_label("Hello"));
    doc.set_env(800.0, 600.0, false, false);
    let frame = doc.frame(0.0);
    assert!(!frame.ops.is_empty());
}
