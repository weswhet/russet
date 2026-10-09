//! `GitHubReleasesInfoProvider`: find a release asset's download URL through
//! the GitHub API. The platform crate makes the requests.
use crate::Result;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary, preferences: Option<&Dictionary>) -> Result<()> {
    autopkg_platform::github::execute_with_preferences(env, preferences)
}
