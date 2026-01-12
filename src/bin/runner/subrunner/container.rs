/// Information about the container used for grading a solution.
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    /// The image which is used to run the container.
    pub podman_image: String,

    /// The name of the container.
    pub podman_container_name: String,

    /// The network to attach to the container.
    pub podman_network_name: String,

    /// The directory inside the container which contains the built solution
    pub internal_build_dir: String,

    /// Path inside the container which the solution dir is mounted to.
    pub mount_solution: String,

    /// Path inside the container which is used to place tests in.
    pub mount_tests: String,

    /// The directory outside the container that solution is mounted from.
    pub external_solution: String,

    /// The directory outside the container that tests are mounted from.
    pub external_tests: String,
}
