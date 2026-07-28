//! Integration test: expand a real document and drive the generated `Doc`.

slab_macro::include_doc!(counter, "tests/fixtures/counter.slab");
slab_macro::include_doc!(list, "tests/fixtures/list.slab");

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

#[test]
fn generated_list_cache_can_be_invalidated() {
	let mut doc = list::Doc::new();
	let rows = [list::RowsItem { value: "First".to_string(), ..Default::default() }];

	assert!(doc.set_rows(&rows));
	doc.invalidate_caches();
	assert!(doc.set_rows(&rows));
}
