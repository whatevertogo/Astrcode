use std::path::PathBuf;

use astrcode_eval::task::loader::TaskLoader;

#[test]
fn core_task_set_loads_successfully() {
    let task_set = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../eval-tasks/task-set.yaml");
    let loaded = TaskLoader::load_task_set(&task_set).expect("core task set should load");
    assert_eq!(loaded.tasks.len(), 3);
    assert!(loaded.warnings.is_empty());
}
