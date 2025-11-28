use crate::WorkbenchDescriptor;
use once_cell::sync::Lazy;
use std::sync::Mutex;

pub static REGISTERED_WORKBENCHES: Lazy<Mutex<Vec<WorkbenchDescriptor>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

#[macro_export]
macro_rules! define_workbenches {
    ($($workbench_type:ty),* $(,)?) => {
        pub fn register_all_workbenches(registry: &mut DocumentService) -> DocumentResult<()> {
            $(
                let workbench = <$workbench_type>::default();
                let descriptor = workbench.descriptor();
                registry.register_workbench(Box::new(workbench))?;
                $crate::registration::REGISTERED_WORKBENCHES.lock().unwrap().push(descriptor);
            )*

            // Sort by label
            $crate::registration::REGISTERED_WORKBENCHES.lock().unwrap().sort_by(|a, b| a.label.cmp(&b.label));

            Ok(())
        }
    };
}
