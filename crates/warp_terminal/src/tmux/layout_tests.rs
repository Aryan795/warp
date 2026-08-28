use super::{LayoutLeaf, LayoutNode, PaneId, SplitStep, parse_window_layout, split_steps};

#[test]
fn parses_a_single_pane() {
    let layout = parse_window_layout("80x24,0,0,3").unwrap();
    assert_eq!(layout.pane_ids(), vec![PaneId::from("%3")]);
}

#[test]
fn parses_checksum_prefixed_side_by_side_split() {
    let layout = parse_window_layout("b25d,80x24,0,0{40x24,0,0,0,39x24,41,0,1}").unwrap();
    assert_eq!(
        layout.pane_ids(),
        vec![PaneId::from("%0"), PaneId::from("%1")]
    );
    match layout {
        LayoutNode::Split {
            horizontal,
            children,
            ..
        } => {
            assert!(horizontal);
            assert_eq!(children.len(), 2);
        }
        LayoutNode::Leaf(_) => panic!("expected split"),
    }
}

#[test]
fn split_steps_emit_one_side_by_side_from_first_leaf() {
    let layout = parse_window_layout("80x24,0,0{40x24,0,0,0,39x24,41,0,1}").unwrap();
    assert_eq!(
        split_steps(&layout),
        vec![SplitStep {
            parent: PaneId::from("%0"),
            new_pane: PaneId::from("%1"),
            side_by_side: true,
        }]
    );
}

#[test]
fn stacked_split_is_not_side_by_side() {
    let layout = parse_window_layout("80x24,0,0[40x12,0,0,0,40x11,0,13,2]").unwrap();
    assert_eq!(
        split_steps(&layout),
        vec![SplitStep {
            parent: PaneId::from("%0"),
            new_pane: PaneId::from("%2"),
            side_by_side: false,
        }]
    );
}

#[test]
fn leaf_geometry_is_preserved() {
    let layout = parse_window_layout("40x12,1,2,7").unwrap();
    match layout {
        LayoutNode::Leaf(LayoutLeaf {
            width,
            height,
            x,
            y,
            pane_id,
        }) => {
            assert_eq!((width, height, x, y), (40, 12, 1, 2));
            assert_eq!(pane_id, PaneId::from("%7"));
        }
        LayoutNode::Split { .. } => panic!("expected leaf"),
    }
}
