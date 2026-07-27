macro_rules! kernel_tests {
    ($($name:ident => $test:path;)*) => {
        $(
            #[test]
            #[allow(non_snake_case)]
            fn $name() {
                $test();
            }
        )*
    };
}

kernel_tests! {
    test_cells__test_alpha_compositing => slab_kernel::test_cells::test_alpha_compositing;
    test_cells__test_gradient_sampling => slab_kernel::test_cells::test_gradient_sampling;
    test_cells__test_grid_dims => slab_kernel::test_cells::test_grid_dims;
    test_cells__test_hairlines_and_borders => slab_kernel::test_cells::test_hairlines_and_borders;
    test_cells__test_quantization_half_even => slab_kernel::test_cells::test_quantization_half_even;
    test_cells__test_serialize => slab_kernel::test_cells::test_serialize;
    test_cells__test_text_and_fill_bg => slab_kernel::test_cells::test_text_and_fill_bg;
    test_cells__test_wide_grapheme_clusters => slab_kernel::test_cells::test_wide_grapheme_clusters;
    test_cells__test_wide_overwrite_cleanup => slab_kernel::test_cells::test_wide_overwrite_cleanup;

    test_color__test_lerp => slab_kernel::test_color::test_lerp;
    test_color__test_math_intrinsics => slab_kernel::test_color::test_math_intrinsics;
    test_color__test_roundtrip => slab_kernel::test_color::test_roundtrip;

    test_ease__test_ease_in => slab_kernel::test_ease::test_ease_in;
    test_ease__test_ease_in_out => slab_kernel::test_ease::test_ease_in_out;
    test_ease__test_ease_out => slab_kernel::test_ease::test_ease_out;
    test_ease__test_linear => slab_kernel::test_ease::test_linear;

    test_edit__test_caret_respects_clusters => slab_kernel::test_edit::test_caret_respects_clusters;
    test_edit__test_coalesced_undo_and_redo_invalidation => slab_kernel::test_edit::test_coalesced_undo_and_redo_invalidation;
    test_edit__test_collapse_selection_on_move => slab_kernel::test_edit::test_collapse_selection_on_move;
    test_edit__test_composition_update_then_commit => slab_kernel::test_edit::test_composition_update_then_commit;
    test_edit__test_field_scroll_offsets_text_and_forces_clip => slab_kernel::test_edit::test_field_scroll_offsets_text_and_forces_clip;
    test_edit__test_host_focus_binds_field_and_rejects_inert => slab_kernel::test_edit::test_host_focus_binds_field_and_rejects_inert;
    test_edit__test_insert_preserves_newlines => slab_kernel::test_edit::test_insert_preserves_newlines;
    test_edit__test_kills_and_word_deletes => slab_kernel::test_edit::test_kills_and_word_deletes;
    test_edit__test_movement_selection_and_word_kill_break_undo_runs => slab_kernel::test_edit::test_movement_selection_and_word_kill_break_undo_runs;
    test_edit__test_overwide_field_stays_clipped_at_zero_scroll => slab_kernel::test_edit::test_overwide_field_stays_clipped_at_zero_scroll;
    test_edit__test_selection_and_words => slab_kernel::test_edit::test_selection_and_words;
    test_edit__test_selection_bands_for_wrapped_two_lines => slab_kernel::test_edit::test_selection_bands_for_wrapped_two_lines;
    test_edit__test_source_line_maps_and_layout_lookup => slab_kernel::test_edit::test_source_line_maps_and_layout_lookup;
    test_edit__test_visual_goal_x_survives_short_line => slab_kernel::test_edit::test_visual_goal_x_survives_short_line;
    test_edit__test_zwj_emoji_is_one_stop => slab_kernel::test_edit::test_zwj_emoji_is_one_stop;

    test_divider__test_disabled_nodes_reject_host_focus_and_tab => slab_kernel::test_divider::test_disabled_nodes_reject_host_focus_and_tab;
    test_divider__test_column_divider_axis_and_cursor => slab_kernel::test_divider::test_column_divider_axis_and_cursor;
    test_divider__test_divider_clamp_honors_every_bound => slab_kernel::test_divider::test_divider_clamp_honors_every_bound;
    test_divider__test_divider_host_restore_and_layout_bounds => slab_kernel::test_divider::test_divider_host_restore_and_layout_bounds;
    test_divider__test_divider_pointer_keyboard_cursor_and_reset => slab_kernel::test_divider::test_divider_pointer_keyboard_cursor_and_reset;
    test_divider__test_divider_neighbors_ignore_sticky_paint_order => slab_kernel::test_divider::test_divider_neighbors_ignore_sticky_paint_order;
    test_divider__test_focus_order_ignores_sticky_promotion_and_duplicate_keys => slab_kernel::test_divider::test_focus_order_ignores_sticky_promotion_and_duplicate_keys;
    test_divider__test_divider_overlay_prunes_with_synthetic_identity => slab_kernel::test_divider::test_divider_overlay_prunes_with_synthetic_identity;
    test_divider__test_divider_release_uses_fresh_layout_clamp => slab_kernel::test_divider::test_divider_release_uses_fresh_layout_clamp;
    test_divider__test_divider_reserves_nonfixed_handle_footprints => slab_kernel::test_divider::test_divider_reserves_nonfixed_handle_footprints;

    test_fmt3__test_fmt3_half_even => slab_kernel::test_fmt3::test_fmt3_half_even;
    test_fmt3__test_fmt3_integers => slab_kernel::test_fmt3::test_fmt3_integers;
    test_fmt3__test_fmt3_negzero => slab_kernel::test_fmt3::test_fmt3_negzero;
    test_fmt3__test_fmt3_trim => slab_kernel::test_fmt3::test_fmt3_trim;

    test_font_register__test_runtime_font_register_overrides_matching_family => slab_kernel::test_font_register::test_runtime_font_register_overrides_matching_family;

    test_graphemes__test_boundary_navigation => slab_kernel::test_graphemes::test_boundary_navigation;
    test_graphemes__test_combining_mark_clusters => slab_kernel::test_graphemes::test_combining_mark_clusters;
    test_graphemes__test_crlf_is_one_cluster => slab_kernel::test_graphemes::test_crlf_is_one_cluster;
    test_graphemes__test_empty_text => slab_kernel::test_graphemes::test_empty_text;
    test_graphemes__test_flag_pairs_split_in_twos => slab_kernel::test_graphemes::test_flag_pairs_split_in_twos;
    test_graphemes__test_variation_selector_attaches => slab_kernel::test_graphemes::test_variation_selector_attaches;
    test_graphemes__test_zwj_family_is_one_cluster => slab_kernel::test_graphemes::test_zwj_family_is_one_cluster;

    test_gesture__test_double_click_suppresses_activate => slab_kernel::test_gesture::test_double_click_suppresses_activate;
    test_gesture__test_blur_and_close_emit_cancelled_drag_end_once => slab_kernel::test_gesture::test_blur_and_close_emit_cancelled_drag_end_once;
    test_gesture__test_continuous_drag_signals_and_release_metadata => slab_kernel::test_gesture::test_continuous_drag_signals_and_release_metadata;
    test_gesture__test_fresh_scene_cancels_missing_drag_source => slab_kernel::test_gesture::test_fresh_scene_cancels_missing_drag_source;
    test_gesture__test_drag_cancel_and_blur_clear_all_gesture_state => slab_kernel::test_gesture::test_drag_cancel_and_blur_clear_all_gesture_state;
    test_gesture__test_drag_release_revalidates_source => slab_kernel::test_gesture::test_drag_release_revalidates_source;
    test_gesture__test_drag_threshold_deepest_drop_and_source_metadata => slab_kernel::test_gesture::test_drag_threshold_deepest_drop_and_source_metadata;
    test_gesture__test_press_and_context_button_semantics => slab_kernel::test_gesture::test_press_and_context_button_semantics;
    test_gesture__test_pruned_drag_source_clears_surviving_drop_state => slab_kernel::test_gesture::test_pruned_drag_source_clears_surviving_drop_state;
    test_gesture__test_secondary_pointer_up_routes_without_releasing_primary_capture => slab_kernel::test_gesture::test_secondary_pointer_up_routes_without_releasing_primary_capture;

    test_hit__test_activation_key_bubbles_to_ancestor => slab_kernel::test_hit::test_activation_key_bubbles_to_ancestor;
    test_hit__test_clip_parent_blocks_outside_hits => slab_kernel::test_hit::test_clip_parent_blocks_outside_hits;
    test_hit__test_disabled_activation_key_is_suppressed => slab_kernel::test_hit::test_disabled_activation_key_is_suppressed;
    test_hit__test_focusables_document_order => slab_kernel::test_hit::test_focusables_document_order;
    test_hit__test_inert_overlay_passes_through => slab_kernel::test_hit::test_inert_overlay_passes_through;
    test_hit__test_quarter_rotation_bbox => slab_kernel::test_hit::test_quarter_rotation_bbox;
    test_hit__test_rotated_clipping_ancestor => slab_kernel::test_hit::test_rotated_clipping_ancestor;
    test_hit__test_rotation_hit_follows_transform => slab_kernel::test_hit::test_rotation_hit_follows_transform;
    test_hit__test_rounded_corner_misses => slab_kernel::test_hit::test_rounded_corner_misses;
    test_hit__test_scroll_clamp_bounds => slab_kernel::test_hit::test_scroll_clamp_bounds;
    test_hit__test_shift_arrow_fast_scrolls => slab_kernel::test_hit::test_shift_arrow_fast_scrolls;
    test_hit__test_synthetic_activation_carries_item_key => slab_kernel::test_hit::test_synthetic_activation_carries_item_key;
    test_hit__test_wheel_routes_main_and_cross_axes_independently => slab_kernel::test_hit::test_wheel_routes_main_and_cross_axes_independently;
    test_hit__test_topmost_wins_in_overlap => slab_kernel::test_hit::test_topmost_wins_in_overlap;
    test_hit__test_trig_values => slab_kernel::test_hit::test_trig_values;

    test_layout__test_hole_report_does_not_override_non_hug_or_non_hole => slab_kernel::test_layout::test_hole_report_does_not_override_non_hug_or_non_hole;
    test_layout__test_hole_size_invalid_and_equal_reports_are_noops => slab_kernel::test_layout::test_hole_size_invalid_and_equal_reports_are_noops;
    test_layout__test_hug_hole_report_is_clamped_by_min_and_max => slab_kernel::test_layout::test_hug_hole_report_is_clamped_by_min_and_max;
    test_layout__test_hug_hole_reported_dimensions_in_both_orientations => slab_kernel::test_layout::test_hug_hole_reported_dimensions_in_both_orientations;
    test_layout__test_hug_hole_unreported_is_zero_across_solves => slab_kernel::test_layout::test_hug_hole_unreported_is_zero_across_solves;

    test_list__test_list_defaults_extend_truncate_and_atomic_rejection => slab_kernel::test_list::test_list_defaults_extend_truncate_and_atomic_rejection;
    test_list__test_list_keyed_reorder_identity_prune_and_key_addressing => slab_kernel::test_list::test_list_keyed_reorder_identity_prune_and_key_addressing;
    test_list__test_list_prop_patch_state_isolation_focus_and_content => slab_kernel::test_list::test_list_prop_patch_state_isolation_focus_and_content;
    test_list__test_list_transition_clock_and_overlay_are_per_item => slab_kernel::test_list::test_list_transition_clock_and_overlay_are_per_item;
    test_list__test_recursive_list_paths_materialization_and_pruning => slab_kernel::test_list::test_recursive_list_paths_materialization_and_pruning;
    test_list__test_recursive_list_defaults_and_reextension_are_clean => slab_kernel::test_list::test_recursive_list_defaults_and_reextension_are_clean;
    test_list__test_virtual_list_window_extent_identity_and_focus => slab_kernel::test_list::test_virtual_list_window_extent_identity_and_focus;
    test_list__test_virtual_list_frame_settle_reveal_and_op_bound => slab_kernel::test_list::test_virtual_list_frame_settle_reveal_and_op_bound;

    test_motion__test_apply_skips_lifted_bindings => slab_kernel::test_motion::test_apply_skips_lifted_bindings;
    test_motion__test_easing_and_cycle_modes => slab_kernel::test_motion::test_easing_and_cycle_modes;
    test_motion__test_lerp_types => slab_kernel::test_motion::test_lerp_types;
    test_motion__test_inst_lift_marks_css_targets => slab_kernel::test_motion::test_inst_lift_marks_css_targets;
    test_motion__test_lift_color_subdivision => slab_kernel::test_motion::test_lift_color_subdivision;
    test_motion__test_lift_easing_remap => slab_kernel::test_motion::test_lift_easing_remap;
    test_motion__test_lift_paint_only_interaction => slab_kernel::test_motion::test_lift_paint_only_interaction;
    test_motion__test_lift_square_full_rotation => slab_kernel::test_motion::test_lift_square_full_rotation;
    test_motion__test_lift_transform_tracks => slab_kernel::test_motion::test_lift_transform_tracks;
    test_motion__test_lift_tuple_scale_track => slab_kernel::test_motion::test_lift_tuple_scale_track;
    test_motion__test_lifts_classification => slab_kernel::test_motion::test_lifts_classification;
    test_motion__test_rgba_swap_involution => slab_kernel::test_motion::test_rgba_swap_involution;
    test_motion__test_tuple_lerp_elementwise => slab_kernel::test_motion::test_tuple_lerp_elementwise;

    test_multiline__test_caret_geometry_honors_padding_and_alignment => slab_kernel::test_multiline::test_caret_geometry_honors_padding_and_alignment;
    test_multiline__test_enter_matrix_and_submit_payload => slab_kernel::test_multiline::test_enter_matrix_and_submit_payload;
    test_multiline__test_fresh_wrapped_layout_scroll_follow_settles => slab_kernel::test_multiline::test_fresh_wrapped_layout_scroll_follow_settles;
    test_multiline__test_horizontal_and_ancestor_scroll_follow => slab_kernel::test_multiline::test_horizontal_and_ancestor_scroll_follow;
    test_multiline__test_kills_undo_and_redo => slab_kernel::test_multiline::test_kills_undo_and_redo;
    test_multiline__test_paste_undoes_in_one_step => slab_kernel::test_multiline::test_paste_undoes_in_one_step;
    test_multiline__test_single_line_text_prefilters_newlines => slab_kernel::test_multiline::test_single_line_text_prefilters_newlines;
    test_multiline__test_visual_arrows_home_end_and_caret_geometry => slab_kernel::test_multiline::test_visual_arrows_home_end_and_caret_geometry;

    test_textm__test_default_advance => slab_kernel::test_textm::test_default_advance;
    test_textm__test_hard_break_long_word => slab_kernel::test_textm::test_hard_break_long_word;
    test_textm__test_hard_newline => slab_kernel::test_textm::test_hard_newline;
    test_textm__test_max_lines => slab_kernel::test_textm::test_max_lines;
    test_textm__test_max_lines_ellipsis_appends => slab_kernel::test_textm::test_max_lines_ellipsis_appends;
    test_textm__test_metrics => slab_kernel::test_textm::test_metrics;
    test_textm__test_nowrap_clipped_no_ellipsis => slab_kernel::test_textm::test_nowrap_clipped_no_ellipsis;
    test_textm__test_nowrap_ellipsis => slab_kernel::test_textm::test_nowrap_ellipsis;
    test_textm__test_wrap_basic => slab_kernel::test_textm::test_wrap_basic;
    test_textm__test_wrap_nbsp_glue => slab_kernel::test_textm::test_wrap_nbsp_glue;

    test_value__test_f64_bits => slab_kernel::test_value::test_f64_bits;
    test_value__test_fmt_u32 => slab_kernel::test_value::test_fmt_u32;
    test_value__test_integer_power_of_two_arithmetic => slab_kernel::test_value::test_integer_power_of_two_arithmetic;
    test_value__test_string_ops => slab_kernel::test_value::test_string_ops;
    test_value__test_utf8_str => slab_kernel::test_value::test_utf8_str;
    test_value__test_tuple_dyn_members_track_params => slab_kernel::test_value::test_tuple_dyn_members_track_params;
    test_value__test_value_decode => slab_kernel::test_value::test_value_decode;

    test_when__test_bool_param_override => slab_kernel::test_when::test_bool_param_override;
    test_when__test_client_code => slab_kernel::test_when::test_client_code;
    test_when__test_client_cond => slab_kernel::test_when::test_client_cond;
    test_when__test_env_conds => slab_kernel::test_when::test_env_conds;
    test_when__test_state_conds => slab_kernel::test_when::test_state_conds;
    test_when__test_theme_cond => slab_kernel::test_when::test_theme_cond;
    test_when__test_wcmp_boundaries => slab_kernel::test_when::test_wcmp_boundaries;
}
