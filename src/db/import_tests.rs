use super::*;

fn imported() -> crate::postman::ImportResult {
    crate::postman::import_collection(include_str!(
        "../../tests/fixtures/import/postman-variables.json"
    ))
    .unwrap()
}

#[test]
fn imported_variables_get_isolated_unique_environments() {
    let db = Database::new_in_memory();
    let existing_id = db.create_environment("Scoped API (Imported)").unwrap();
    db.replace_variables(
        existing_id,
        &[EnvVar {
            enabled: true,
            key: "token".into(),
            value: "keep-me".into(),
        }],
    )
    .unwrap();
    db.set_active_environment_id(Some(existing_id)).unwrap();
    let imported = imported();
    for suffix in [2, 3] {
        let (collection_id, environment) = db
            .insert_collection_import(&imported.collection, "Scoped API", &imported.variables)
            .unwrap();
        let environment = environment.expect("variables need a persisted environment");
        assert_eq!(environment.name, format!("Scoped API (Imported {suffix})"));
        assert_eq!(environment.variables.len(), 5);
        assert!(!environment.variables[2].enabled);
        assert_eq!(environment.variables[3].value, "10");
        assert_eq!(environment.variables[4].value, "true");
        let collections = db.load_collections().unwrap();
        let request = &collections
            .iter()
            .find(|c| c.id == collection_id)
            .unwrap()
            .requests[0]
            .request;
        let vars = environment
            .variables
            .iter()
            .filter(|v| v.enabled)
            .map(|v| (v.key.clone(), v.value.clone()))
            .collect();
        let resolved = crate::variables::substitute_request(request, &vars);
        assert_eq!(
            resolved.auth.effective_url(&resolved.url),
            "https://example.test/items?count=10&api_key=imported-token"
        );
    }
    let environments = db.load_environments().unwrap();
    assert_eq!(environments.len(), 3);
    assert_eq!(environments[0].variables[0].value, "keep-me");
    assert_eq!(db.get_active_environment_id().unwrap(), Some(existing_id));
}

#[test]
fn import_without_variables_does_not_create_an_environment() {
    let db = Database::new_in_memory();
    let imported = imported();
    let (_, environment) = db
        .insert_collection_import(&imported.collection, "No variables", &[])
        .unwrap();
    assert!(environment.is_none());
    assert!(db.load_environments().unwrap().is_empty());
}

#[test]
fn import_transaction_rolls_back_collection_if_environment_write_fails() {
    let db = Database::new_in_memory();
    db.call(|conn| {
        conn.execute_batch("CREATE TRIGGER reject_import BEFORE INSERT ON env_variables BEGIN SELECT RAISE(ABORT, 'test failure'); END;")?;
        Ok(())
    }).unwrap();
    let imported = imported();
    assert!(
        db.insert_collection_import(&imported.collection, "Failed", &imported.variables)
            .is_err()
    );
    assert!(db.load_collections().unwrap().is_empty());
    assert!(db.load_environments().unwrap().is_empty());
}
