//! Exercises departments and, above all, who can see whom.
//!
//! The visibility rules are the point of the milestone, so most of these tests
//! are about what a list does *not* contain. A leak here is silent: nobody
//! notices an extra row, whereas a missing one is reported the same afternoon.
//!
//! Skipped unless `DATABASE_URL` is set; CI runs them with a Postgres service.

mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::TestServer;

const PASSWORD: &str = "correct horse battery staple";

async fn as_admin(server: &TestServer) -> String {
    server.add_admin("boss@example.test", PASSWORD).await;
    let (_, cookie, _) = server.login("boss@example.test", PASSWORD).await;
    cookie.expect("the admin should sign in")
}

/// Creates a person and returns their id.
async fn person(server: &TestServer, admin: &str, email: &str, role: &str) -> String {
    let (status, _, body) = server
        .post_with_cookie("/api/v1/users", Some(admin), json!({"email": email, "role": role, "password": PASSWORD}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{email}: {body}");
    body["id"].as_str().expect("an id is returned").to_string()
}

/// Creates a department and returns its id.
async fn department(server: &TestServer, admin: &str, name: &str, manager_id: Option<&str>) -> String {
    let (status, _, body) = server
        .post_with_cookie("/api/v1/departments", Some(admin), json!({"name": name, "manager_id": manager_id}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{name}: {body}");
    body["id"].as_str().expect("an id is returned").to_string()
}

async fn assign(server: &TestServer, admin: &str, user: &str, department_id: Option<&str>) -> StatusCode {
    let (status, _, _) = server
        .put_with_cookie(
            &format!("/api/v1/users/{user}/department"),
            Some(admin),
            json!({"department_id": department_id}),
        )
        .await;
    status
}

/// The email addresses in a `GET /users` response, sorted.
fn emails(list: &Value) -> Vec<String> {
    let mut emails: Vec<String> = list
        .as_array()
        .expect("a list of users")
        .iter()
        .map(|row| row["email"].as_str().unwrap_or_default().to_string())
        .collect();
    emails.sort();
    emails
}

#[tokio::test]
async fn a_manager_sees_their_department_and_nobody_else() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;

    let manager_id = person(&server, &admin, "manager@example.test", "manager").await;
    let other_manager_id = person(&server, &admin, "other@example.test", "manager").await;
    let mine = person(&server, &admin, "mine@example.test", "employee").await;
    let theirs = person(&server, &admin, "theirs@example.test", "employee").await;
    let unfiled = person(&server, &admin, "unfiled@example.test", "employee").await;

    let engineering = department(&server, &admin, "Engineering", Some(&manager_id)).await;
    let sales = department(&server, &admin, "Sales", Some(&other_manager_id)).await;
    assign(&server, &admin, &mine, Some(&engineering)).await;
    assign(&server, &admin, &theirs, Some(&sales)).await;
    assign(&server, &admin, &manager_id, Some(&engineering)).await;

    let (_, manager_cookie, _) = server.login("manager@example.test", PASSWORD).await;
    let (status, list) = server.get_with_cookie("/api/v1/users", manager_cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        emails(&list),
        vec!["manager@example.test", "mine@example.test"],
        "a manager sees their own department and themselves - not Sales, not the unfiled, not the admin"
    );
    assert!(!emails(&list).contains(&"unfiled@example.test".to_string()));
    let _ = unfiled;

    // The admin still sees everyone; narrowing is a manager's rule, not a
    // change to what administration means.
    let (_, all) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    assert_eq!(all.as_array().unwrap().len(), 7, "six created plus the seeded agent account");
}

#[tokio::test]
async fn a_manager_without_a_department_still_sees_themselves() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    person(&server, &admin, "manager@example.test", "manager").await;

    // A manager who runs nothing yet must not get an empty page that looks
    // broken - their own row is the one thing they can always account for.
    let (_, cookie, _) = server.login("manager@example.test", PASSWORD).await;
    let (status, list) = server.get_with_cookie("/api/v1/users", cookie.as_deref()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(emails(&list), vec!["manager@example.test"]);
}

#[tokio::test]
async fn an_unfiled_person_is_visible_to_the_admin_alone() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let manager_id = person(&server, &admin, "manager@example.test", "manager").await;
    person(&server, &admin, "newcomer@example.test", "employee").await;
    let engineering = department(&server, &admin, "Engineering", Some(&manager_id)).await;
    assign(&server, &admin, &manager_id, Some(&engineering)).await;

    // The deliberate default: someone nobody has filed yet is missing from
    // every manager's list, which is noticed, rather than present in all of
    // them, which is not.
    let (_, cookie, _) = server.login("manager@example.test", PASSWORD).await;
    let (_, list) = server.get_with_cookie("/api/v1/users", cookie.as_deref()).await;
    assert!(!emails(&list).contains(&"newcomer@example.test".to_string()));

    let (_, all) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    assert!(emails(&all).contains(&"newcomer@example.test".to_string()), "the admin must see who is unfiled");
}

#[tokio::test]
async fn moving_someone_moves_who_can_see_them() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let manager_id = person(&server, &admin, "manager@example.test", "manager").await;
    let mover = person(&server, &admin, "mover@example.test", "employee").await;
    let engineering = department(&server, &admin, "Engineering", Some(&manager_id)).await;
    assign(&server, &admin, &manager_id, Some(&engineering)).await;
    assign(&server, &admin, &mover, Some(&engineering)).await;

    let (_, cookie, _) = server.login("manager@example.test", PASSWORD).await;
    let (_, before) = server.get_with_cookie("/api/v1/users", cookie.as_deref()).await;
    assert!(emails(&before).contains(&"mover@example.test".to_string()));

    // Removing someone from a department is `null`, not a delete: they keep
    // their history and simply stop being anyone's report.
    assert_eq!(assign(&server, &admin, &mover, None).await, StatusCode::NO_CONTENT);

    let (_, after) = server.get_with_cookie("/api/v1/users", cookie.as_deref()).await;
    assert!(
        !emails(&after).contains(&"mover@example.test".to_string()),
        "they must leave the manager's list"
    );
    let (_, all) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    assert!(emails(&all).contains(&"mover@example.test".to_string()), "but not the company");
}

#[tokio::test]
async fn deleting_a_department_keeps_its_people() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let manager_id = person(&server, &admin, "manager@example.test", "manager").await;
    let member = person(&server, &admin, "member@example.test", "employee").await;
    let engineering = department(&server, &admin, "Engineering", Some(&manager_id)).await;
    assign(&server, &admin, &member, Some(&engineering)).await;

    let (status, _, _) = server.delete_with_cookie(&format!("/api/v1/departments/{engineering}"), Some(&admin)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Deleting a group must never delete the people in it.
    let (_, all) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    assert!(emails(&all).contains(&"member@example.test".to_string()));
    let row = all.as_array().unwrap().iter().find(|row| row["email"] == "member@example.test").unwrap();
    assert!(row["department_id"].is_null(), "they are unfiled, not gone: {row}");
    let _ = member;
}

#[tokio::test]
async fn a_department_can_be_between_managers() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let manager_id = person(&server, &admin, "manager@example.test", "manager").await;
    let engineering = department(&server, &admin, "Engineering", Some(&manager_id)).await;

    // An explicit null clears the head; an omitted field leaves it alone. Both
    // are real requests and must not be the same one.
    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/departments/{engineering}"), Some(&admin), json!({"manager_id": null}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, list) = server.get_with_cookie("/api/v1/departments", Some(&admin)).await;
    let row = list.as_array().unwrap().iter().find(|row| row["name"] == "Engineering").unwrap();
    assert!(row["manager_id"].is_null(), "the department is between heads: {row}");

    let (status, _, _) = server
        .patch_with_cookie(&format!("/api/v1/departments/{engineering}"), Some(&admin), json!({"name": "Platform"}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = server.get_with_cookie("/api/v1/departments", Some(&admin)).await;
    let row = list.as_array().unwrap().iter().find(|row| row["name"] == "Platform").unwrap();
    assert!(row["manager_id"].is_null(), "a rename must not resurrect the old head: {row}");
}

#[tokio::test]
async fn an_employee_cannot_run_a_department() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let worker = person(&server, &admin, "worker@example.test", "employee").await;

    // They would see nothing of it - `GET /users` admits managers and admins
    // only - so the department would silently have no working head.
    let (status, _, message) = server
        .post_with_cookie("/api/v1/departments", Some(&admin), json!({"name": "Engineering", "manager_id": worker}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");
    assert!(
        message["error"].as_str().unwrap_or_default().contains("make them a manager first"),
        "the message should say what to do: {message}"
    );
}

#[tokio::test]
async fn departments_are_administered_by_an_admin_and_read_by_a_manager() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    person(&server, &admin, "manager@example.test", "manager").await;
    person(&server, &admin, "worker@example.test", "employee").await;
    let engineering = department(&server, &admin, "Engineering", None).await;

    let (_, manager, _) = server.login("manager@example.test", PASSWORD).await;
    let (status, _) = server.get_with_cookie("/api/v1/departments", manager.as_deref()).await;
    assert_eq!(status, StatusCode::OK, "a manager needs to know the shape of the company");

    let (status, _, _) = server
        .post_with_cookie("/api/v1/departments", manager.as_deref(), json!({"name": "Sales"}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "but not to reorganise it");
    let (status, _, _) = server
        .delete_with_cookie(&format!("/api/v1/departments/{engineering}"), manager.as_deref())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (_, worker, _) = server.login("worker@example.test", PASSWORD).await;
    let (status, _) = server.get_with_cookie("/api/v1/departments", worker.as_deref()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an employee has no business here at all");
}

#[tokio::test]
async fn two_departments_cannot_share_a_name() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    department(&server, &admin, "Engineering", None).await;

    // People say department names out loud; two called "Engineering" is a
    // support call waiting to happen.
    let (status, _, message) = server
        .post_with_cookie("/api/v1/departments", Some(&admin), json!({"name": "ENGINEERING"}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{message}");

    let (status, _, _) = server.post_with_cookie("/api/v1/departments", Some(&admin), json!({"name": "  "})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a nameless department is not one");
}

#[tokio::test]
async fn assigning_to_a_department_that_does_not_exist_is_refused() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let worker = person(&server, &admin, "worker@example.test", "employee").await;
    let missing = uuid::Uuid::new_v4().to_string();

    let (status, _, message) = server
        .put_with_cookie(&format!("/api/v1/users/{worker}/department"), Some(&admin), json!({"department_id": missing}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{message}");

    let missing_user = uuid::Uuid::new_v4().to_string();
    let (status, _, _) = server
        .put_with_cookie(
            &format!("/api/v1/users/{missing_user}/department"),
            Some(&admin),
            json!({"department_id": null}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_list_carries_the_department_so_a_page_needs_one_request() {
    let Some(server) = TestServer::start().await else { return };
    let admin = as_admin(&server).await;
    let worker = person(&server, &admin, "worker@example.test", "employee").await;
    let engineering = department(&server, &admin, "Engineering", None).await;
    assign(&server, &admin, &worker, Some(&engineering)).await;

    let (_, list) = server.get_with_cookie("/api/v1/users", Some(&admin)).await;
    let row = list.as_array().unwrap().iter().find(|row| row["email"] == "worker@example.test").unwrap();
    assert_eq!(row["department"], "Engineering", "the name travels with the row: {row}");
    assert_eq!(row["department_id"], engineering);

    let (_, departments) = server.get_with_cookie("/api/v1/departments", Some(&admin)).await;
    let row = departments.as_array().unwrap().iter().find(|row| row["name"] == "Engineering").unwrap();
    assert_eq!(row["members"], 1, "and the count an admin actually looks at: {row}");
}
