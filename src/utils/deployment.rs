//! Multi-Region Deployment (Task 50)

pub struct DeploymentRegion {
    pub name: String,
    pub url: String,
    pub active: bool,
}

pub struct MultiRegionDeployment {
    regions: Vec<DeploymentRegion>,
}

impl MultiRegionDeployment {
    pub fn new() -> Self {
        Self { regions: Vec::new() }
    }

    pub fn add_region(&mut self, name: &str, url: &str) {
        self.regions.push(DeploymentRegion {
            name: name.to_string(),
            url: url.to_string(),
            active: true,
        });
    }

    pub fn active_regions(&self) -> Vec<&DeploymentRegion> {
        self.regions.iter().filter(|r| r.active).collect()
    }
}
