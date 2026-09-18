use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn changed_input_lengths_fail_without_replacing_destination() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "ndstool-plan-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("data")).unwrap();
    let input = root.join("data/file");
    let destination = root.join("output.nds");
    fs::write(&destination, b"previous ROM").unwrap();
    for length in [0, 2, 6] {
        fs::write(&input, [1; 4]).unwrap();
        let plan = plan_image(&root.join("data"), 512, 0, LayoutMode::Stable).unwrap();
        fs::write(&input, vec![2; length]).unwrap();
        let (staged, mut file) = crate::output::Output::new(&destination).unwrap();
        assert!(plan.write_serial(&mut file, 512).is_err());
        drop(file);
        assert!(plan.write_parallel(&staged.path, 512, 2).is_err());
        drop(staged);
        assert_eq!(fs::read(&destination).unwrap(), b"previous ROM");
    }
    fs::remove_dir_all(root).unwrap();
}
