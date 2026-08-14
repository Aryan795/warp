use super::*;

/// Builds a minimal gzipped pprof profile containing one sample per entry in
/// `stacks`. Each stack is a leaf-first list of function names; a fresh
/// `Location`/`Function` pair is created for every frame (no de-duplication,
/// which keeps the test setup simple and is representative of the worst
/// case for this logic). Every sample carries a single value of `1`.
fn build_profile(stacks: &[&[&str]]) -> Vec<u8> {
    let mut string_table = vec![String::new()];
    let mut functions = Vec::new();
    let mut locations = Vec::new();
    let mut samples = Vec::new();
    let mut next_id = 1u64;

    for stack in stacks {
        let mut location_id = Vec::new();
        for &name in stack.iter() {
            let name_index = string_table.len() as i64;
            string_table.push(name.to_string());

            let id = next_id;
            next_id += 1;
            functions.push(Function {
                id,
                name: name_index,
                system_name: 0,
                filename: 0,
                start_line: 0,
            });
            locations.push(Location {
                id,
                mapping_id: 0,
                address: 0,
                line: vec![Line {
                    function_id: id,
                    line: 0,
                    column: 0,
                }],
                is_folded: false,
            });
            location_id.push(id);
        }
        samples.push(Sample {
            location_id,
            value: vec![1],
            label: Vec::new(),
        });
    }

    let profile = Profile {
        sample_type: vec![ValueType { r#type: 0, unit: 0 }],
        sample: samples,
        mapping: Vec::new(),
        location: locations,
        function: functions,
        string_table,
        drop_frames: 0,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: 0,
        period_type: None,
        period: 0,
        comment: Vec::new(),
        default_sample_type: 0,
        doc_url: 0,
    };

    let mut encoded = Vec::new();
    profile.encode(&mut encoded).unwrap();
    gzip(&encoded).unwrap()
}

/// Decodes a gzipped pprof profile and returns, for each sample, the
/// leaf-first list of resolved function names -- the same shape the test
/// inputs are constructed from -- for easy comparison.
fn decode_stacks(gzipped_profile: &[u8]) -> Vec<Vec<String>> {
    let raw = gunzip(gzipped_profile).unwrap();
    let profile = Profile::decode(raw.as_slice()).unwrap();

    profile
        .sample
        .iter()
        .map(|sample| {
            sample
                .location_id
                .iter()
                .map(|location_id| {
                    let location = profile
                        .location
                        .iter()
                        .find(|location| location.id == *location_id)
                        .unwrap();
                    let function_id = location.line[0].function_id;
                    let function = profile
                        .function
                        .iter()
                        .find(|function| function.id == function_id)
                        .unwrap();
                    profile.string_table[function.name as usize].clone()
                })
                .collect()
        })
        .collect()
}

fn decode_values(gzipped_profile: &[u8]) -> Vec<Vec<i64>> {
    let raw = gunzip(gzipped_profile).unwrap();
    let profile = Profile::decode(raw.as_slice()).unwrap();
    profile.sample.iter().map(|s| s.value.clone()).collect()
}

#[test]
fn strips_prologue_to_first_application_frame() {
    let stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "prof_alloc_prep",
        "imalloc_body",
        "imalloc",
        "_rjem_je_malloc_default",
        "app::do_work",
        "app::main",
    ];
    let profile = build_profile(&[stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    assert_eq!(
        decode_stacks(&stripped),
        vec![vec!["app::do_work".to_string(), "app::main".to_string()]]
    );
    // Sample values must be preserved exactly.
    assert_eq!(decode_values(&stripped), decode_values(&profile));
}

#[test]
fn strips_varying_length_prologue_due_to_inlining() {
    // Same logical prologue, but `prof_alloc_prep` is a distinct frame in
    // the first sample and inlined away (absent) in the second -- as
    // observed across the two Sentry events on this ticket. Both should
    // still end up with the same application leaf.
    let long_stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "prof_alloc_prep",
        "imalloc_body",
        "app::do_work",
    ];
    let short_stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "imalloc_body",
        "app::do_work",
    ];
    let profile = build_profile(&[long_stack, short_stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    let stacks = decode_stacks(&stripped);
    assert_eq!(stacks[0], vec!["app::do_work".to_string()]);
    assert_eq!(stacks[1], vec!["app::do_work".to_string()]);
}

#[test]
fn preserves_allocator_symbol_deeper_in_the_stack() {
    // An allocator-looking frame that shows up *after* application code
    // (e.g. the app itself calling into an allocator-adjacent helper deeper
    // in the stack) must never be stripped, since stripping only applies to
    // the leading run from the leaf.
    let stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "imalloc_body",
        "app::allocate_buffer",
        "prof_helper_used_by_app",
        "app::main",
    ];
    let profile = build_profile(&[stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    assert_eq!(
        decode_stacks(&stripped),
        vec![vec![
            "app::allocate_buffer".to_string(),
            "prof_helper_used_by_app".to_string(),
            "app::main".to_string(),
        ]]
    );
}

#[test]
fn leaves_all_allocator_stack_untouched() {
    let stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "prof_alloc_prep",
        "imalloc_body",
        "imalloc",
        "_rjem_je_malloc_default",
    ];
    let profile = build_profile(&[stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    // The sample must never be emptied, even though every frame matches.
    assert_eq!(
        decode_stacks(&stripped),
        vec![stack.iter().map(|s| s.to_string()).collect::<Vec<_>>()]
    );
}

#[test]
fn is_allocator_symbol_matches_documented_patterns() {
    for name in [
        "_rjem_je_prof_backtrace",
        "_rjem_je_malloc_default",
        "_rjem_realloc",
        "imalloc",
        "imalloc_body",
        "imalloc_no_sample",
        "prof_alloc_prep",
        "prof_tctx_create",
        "prof_gctx_create",
        "malloc_default",
        "calloc_default",
        "realloc_default",
        "posix_memalign_default",
    ] {
        assert!(is_allocator_symbol(name), "expected {name} to match");
    }

    for name in ["app::do_work", "main", "malloc", "std::vec::Vec::push"] {
        assert!(!is_allocator_symbol(name), "expected {name} to not match");
    }
}
