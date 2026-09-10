use crate::{
    api::audit::{client_ip, log_audit, new_entry},
    db::{get_one, insert},
    entities::{employee as Employee, partner as Partner, state as State, user as User},
    models::Role,
};
use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub mail: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct AuthResponse {
    pub id: Uuid,
    pub mail: String,
    pub name: String,
    pub role: Role,
}

#[utoipa::path(
    post,
    path = "/api/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Successfully authenticated", content_type = "application/json", body = AuthResponse),
        (status = 401, description = "Invalid email or password"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/login")]
pub async fn login(
    req: HttpRequest,
    body: web::Json<LoginRequest>,
    db: web::Data<DatabaseConnection>,
) -> impl Responder {
    let ip = client_ip(&req);
    let query = User::Entity::find().filter(User::Column::Mail.eq(&body.mail));

    match get_one(&**db, query).await {
        Ok(Some(user)) => {
            if user.password != body.password {
                let entry = new_entry(
                    Some(user.id),
                    Some(format!("{:?}", user.role)),
                    "login_failed",
                    Some("user".to_string()),
                    Some(user.id.to_string()),
                    Some(serde_json::json!({ "reason": "wrong_password", "mail": body.mail })),
                    ip,
                );
                if let Err(e) = log_audit(db.get_ref(), entry).await {
                    log::error!("audit log failed: {e}");
                }
                return HttpResponse::Unauthorized().body("Wrong password.");
            }

            match State::Entity::find_by_id(user.id).one(db.get_ref()).await {
                Ok(Some(state)) if state.state != "active" => {
                    let base_message = match state.state.as_str() {
                        "waiting_activation" => {
                            "Votre compte est en attente de validation par un administrateur."
                        }
                        "suspended" => "Votre compte a été suspendu.",
                        "rejected" => "Votre demande d'inscription a été refusée.",
                        _ => "Votre compte n'est pas actif.",
                    };
                    let message = match (state.state.as_str(), &state.reason) {
                        ("rejected" | "suspended", Some(reason)) if !reason.is_empty() => {
                            format!("{base_message} Motif : {reason}")
                        }
                        _ => base_message.to_string(),
                    };
                    return HttpResponse::Forbidden().body(message);
                }
                Ok(_) => {}
                Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
            }

            HttpResponse::Ok().json(AuthResponse {
                id: user.id,
                mail: user.mail.clone(),
                name: user.name.clone(),
                role: user.role.into(),
            })
        }
        Ok(None) => {
            let entry = new_entry(
                None,
                None,
                "login_failed",
                Some("user".to_string()),
                None,
                Some(serde_json::json!({ "reason": "unknown_mail", "mail": body.mail })),
                ip,
            );
            if let Err(e) = log_audit(db.get_ref(), entry).await {
                log::error!("audit log failed: {e}");
            }
            HttpResponse::Unauthorized().finish()
        }
        Err(e) => HttpResponse::InternalServerError().body(e.to_string()),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub mail: String,
    pub name: String,
    pub password: String,
    pub role: Role,
    pub siren: Option<i32>,
    pub social_object: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/register",
    request_body = RegisterRequest,
    responses(
        (status = 200, description = "Successfully registered", content_type = "application/json", body = AuthResponse),
        (status = 400, description = "Invalid role or request data"),
        (status = 401, description = "Invalid email or password"),
        (status = 409, description = "Email already in use"),
        (status = 500, description = "Internal server error")
    )
)]
#[post("/register")]
pub async fn register(
    req: HttpRequest,
    body: web::Json<RegisterRequest>,
    db: web::Data<DatabaseConnection>,
) -> impl Responder {
    let ip = client_ip(&req);
    let query = User::Entity::find().filter(User::Column::Mail.eq(&body.mail));

    match get_one(db.get_ref(), query).await {
        Ok(Some(_)) => return HttpResponse::Conflict().finish(),
        Ok(None) => {}
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    }

    let role = match body.role {
        Role::Admin => return HttpResponse::BadRequest().body("Wrong role."),
        r => r,
    };

    let user = match crate::models::User::new(
        body.mail.clone(),
        body.name.clone(),
        body.password.clone(),
        role,
    ) {
        Ok(user) => user,
        Err(e) => {
            return HttpResponse::InternalServerError().body(e.to_string());
        }
    };

    let entity = User::ActiveModel::from(User::Model::from(user));

    let txn = match db.get_ref().begin().await {
        Ok(txn) => txn,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let inserted_user = match insert::<User::Entity, _>(&txn, entity).await {
        Ok(user) => user,
        Err(e) => {
            let _ = txn.rollback().await;
            return HttpResponse::InternalServerError().body(e.to_string());
        }
    };

    let role_record: Result<(), DbErr> = match role {
        Role::Manant => {
            let employee_model = Employee::ActiveModel {
                id: ActiveValue::Set(inserted_user.id),
                balance: ActiveValue::Set(Some(0.0)),
                ..Default::default()
            };
            let state_model = State::ActiveModel {
                id: ActiveValue::Set(inserted_user.id),
                state: ActiveValue::Set("active".to_string()),
                reason: ActiveValue::Set(Some("Ok".to_string())),
                modified_at: ActiveValue::Set(chrono::Utc::now().timestamp()),
            };

            match insert::<Employee::Entity, _>(&txn, employee_model).await {
                Ok(_) => match insert::<State::Entity, _>(&txn, state_model).await {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        Role::Partner => {
            let partner_model = Partner::ActiveModel {
                id: ActiveValue::Set(inserted_user.id),
                siren: ActiveValue::Set(body.siren),
                social_obj: ActiveValue::Set(body.social_object.clone()),
                verification: ActiveValue::Set(Some(false)),
                ..Default::default()
            };
            let state_model = State::ActiveModel {
                id: ActiveValue::Set(inserted_user.id),
                state: ActiveValue::Set("waiting_activation".to_string()),
                reason: ActiveValue::Set(Some("Waiting for verification".to_string())),
                modified_at: ActiveValue::Set(chrono::Utc::now().timestamp()),
            };

            match insert::<Partner::Entity, _>(&txn, partner_model).await {
                Ok(_) => match insert::<State::Entity, _>(&txn, state_model).await {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
        Role::Admin => unreachable!("Admin role is rejected above"),
    };

    if let Err(e) = role_record {
        let _ = txn.rollback().await;
        return HttpResponse::InternalServerError().body(e.to_string());
    }

    if let Err(e) = txn.commit().await {
        return HttpResponse::InternalServerError().body(e.to_string());
    }

    let entry = new_entry(
        Some(inserted_user.id),
        Some(format!("{:?}", inserted_user.role)),
        "account_created",
        Some("user".to_string()),
        Some(inserted_user.id.to_string()),
        Some(serde_json::json!({ "mail": inserted_user.mail, "role": format!("{:?}", inserted_user.role) })),
        ip,
    );
    if let Err(e) = log_audit(db.get_ref(), entry).await {
        log::error!("audit log failed: {e}");
    }

    HttpResponse::Ok().json(AuthResponse {
        id: inserted_user.id,
        mail: inserted_user.mail,
        name: inserted_user.name,
        role: inserted_user.role.into(),
    })
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/auth").service(login).service(register));
}
