//! Azure DevOps の同一組織に対する認証情報を、1 回の並列処理内で共有する。

use std::collections::HashMap;
use std::sync::OnceLock;

pub(super) struct OrganizationValues<T> {
    values: HashMap<String, OnceLock<T>>,
}

impl<T> OrganizationValues<T> {
    pub(super) fn new<'a>(organizations: impl IntoIterator<Item = &'a str>) -> Self {
        let values = organizations
            .into_iter()
            .map(|organization| (organization.trim().to_ascii_lowercase(), OnceLock::new()))
            .collect();
        Self { values }
    }

    pub(super) fn get_or_init(&self, organization: &str, init: impl FnOnce() -> T) -> Option<&T> {
        self.values
            .get(&organization.trim().to_ascii_lowercase())
            .map(|value| value.get_or_init(init))
    }
}

#[cfg(test)]
mod tests {
    use super::OrganizationValues;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn same_organization_initializes_its_value_only_once() {
        let values = OrganizationValues::new(["Contoso", " contoso ", "Fabrikam"]);
        let calls = AtomicUsize::new(0);

        let first = values
            .get_or_init("CONTOSO", || {
                calls.fetch_add(1, Ordering::Relaxed);
                "pat"
            })
            .unwrap();
        let second = values
            .get_or_init("contoso", || {
                calls.fetch_add(1, Ordering::Relaxed);
                "other"
            })
            .unwrap();

        assert_eq!(*first, "pat");
        assert_eq!(*second, "pat");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn different_organizations_initialize_independently() {
        let values = OrganizationValues::new(["Contoso", "Fabrikam"]);

        assert_eq!(values.get_or_init("contoso", || 1), Some(&1));
        assert_eq!(values.get_or_init("fabrikam", || 2), Some(&2));
    }
}
