use super::*;

#[test]
fn the_bench_its_tools_and_its_joints_name_icons_in_the_set() {
    let wb = AssemblyWorkbench::default();
    assert!(ui_kit::icon::exists(wb.descriptor().icon));
    let mut context = WorkbenchContext::default();
    wb.configure(&mut context);
    for tool in context.tools() {
        let icon = tool.icon.expect("every tool has an icon");
        assert!(ui_kit::icon::exists(icon), "{icon}");
    }
    for tool in JointTool::ALL {
        let kind = tool.joint(
            &Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
            &Rigid::from(BodyPlacement::default()),
            &Anchor::Plane {
                point: [0.0; 3],
                normal: [0.0, 0.0, 1.0],
            },
            &Rigid::from(BodyPlacement::default()),
            1.0,
        );
        assert!(ui_kit::icon::exists(kind.icon()), "{}", kind.icon());
        assert!(ui_kit::icon::exists(tool.icon()), "{}", tool.icon());
        assert_eq!(JointTool::of_kind(&kind), Some(tool));
    }
    assert!(ui_kit::icon::exists(JointKind::Ground.icon()));
}
