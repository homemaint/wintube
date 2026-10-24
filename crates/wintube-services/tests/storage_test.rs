use wintube_services::StorageEngine;

#[tokio::test]
async fn test_storage_engine_crud() {
    let storage = StorageEngine::in_memory().expect("Failed to create in-memory db");

    // Record progress
    storage
        .update_playback_progress(
            "dQw4w9WgXcQ",
            "Rick Astley - Never Gonna Give You Up",
            "UCuAXFkgsw1L7xaCfnd5JJOw",
            "Rick Astley",
            212,
            45,
        )
        .expect("Failed to update progress");

    let resume_pos = storage.get_resume_position("dQw4w9WgXcQ").expect("Failed to get resume pos");
    assert_eq!(resume_pos, Some(45));

    let history = storage.get_recent_history(10).expect("Failed to get history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].title, "Rick Astley - Never Gonna Give You Up");

    // Subscriptions
    storage.add_subscription("UCuAXFkgsw1L7xaCfnd5JJOw", "Rick Astley", None).expect("Sub failed");
    let subs = storage.get_subscriptions().expect("Failed to get subs");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].channel_name, "Rick Astley");
}
