//! A backup made on one install restores on another: profiles with their content, the
//! settings, and nothing twice.

use kova_engine::Engine;

const LINK: &str = "vless://00000000-0000-4000-8000-000000000000@203.0.113.10:443?security=reality&sni=example.com&pbk=AAAA&sid=01&type=tcp#Test%20node";

#[tokio::test]
async fn backup_moves_profiles_and_settings() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let from = Engine::new(a.path()).unwrap();
    from.add_profile(LINK, Some("Mine".into())).await.unwrap();
    let mut s = from.settings();
    s.language = "fa".into();
    from.update_settings(s).await.unwrap();
    let text = from.export_backup(Some("secret")).unwrap();

    let to = Engine::new(b.path()).unwrap();
    let device = to.settings().device_id;
    let backup = kova_engine::backup::read(&text, Some("secret")).unwrap();
    assert_eq!(to.import_backup(backup.clone(), true).await.unwrap(), (1, 0));

    let profiles = to.profiles();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Mine");
    assert_eq!(to.profile_content(&profiles[0].id).unwrap(), LINK);
    assert_eq!(to.settings().active_profile.as_deref(), Some(profiles[0].id.as_str()));
    assert_eq!(to.settings().language, "fa");
    // Each install keeps its own device ID, which providers count devices by.
    assert_eq!(to.settings().device_id, device);

    // The same backup again adds nothing.
    assert_eq!(to.import_backup(backup, true).await.unwrap(), (0, 1));
    assert_eq!(to.profiles().len(), 1);
}
