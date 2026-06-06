
pub(crate) fn prepare_dependencies_for_build(
    project_toml: &LocalProjectToml,
    workspace_tree: &Tree,
    dependencies_to_install: &mut Vec<PackageInstallSpec>,
    build_dependencies_to_install: &mut Vec<PackageInstallSpec>,
) {
    let dependencies = project_toml
        .dependencies()
        .current_platform()
        .iter()
        .cloned()
        .collect_vec();

    let build_dependencies = project_toml
        .build_dependencies()
        .current_platform()
        .iter()
        .cloned()
        .collect_vec();
    dependencies
        .into_iter()
        .filter(|dep| {
            workspace_tree
                .match_rocks(dep.package_req())
                .is_ok_and(|rock_match| !rock_match.is_found())
        })
        .map(|dep| {
            PackageInstallSpec::new(dep.clone().into_package_req(), tree::EntryType::Entrypoint)
                .pin(*dep.pin())
                .opt(*dep.opt())
                .maybe_source(dep.source().clone())
                .build()
        })
        .for_each(|dep| dependencies_to_install.push(dep));

    build_dependencies
        .into_iter()
        .filter(|dep| {
            workspace_tree
                .match_rocks(dep.package_req())
                .is_ok_and(|rock_match| !rock_match.is_found())
        })
        .map(|dep| {
            PackageInstallSpec::new(dep.clone().into_package_req(), tree::EntryType::Entrypoint)
                .pin(*dep.pin())
                .opt(*dep.opt())
                .maybe_source(dep.source().clone())
                .build()
        })
        .for_each(|dep| build_dependencies_to_install.push(dep));
}

