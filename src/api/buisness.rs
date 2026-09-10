use actix_web::{HttpRequest, HttpResponse, Responder, get, post, web};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult,
    QueryFilter, QuerySelect, RelationTrait, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    db::{get_by_id, get_one, update},
    entities::{employee, partner, state, transaction, user},
};

#[allow(unused)]
#[derive(ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, ToSchema, FromQueryResult)]
pub struct PendingPartner {
    pub id: Uuid,
    pub name: String,
    pub mail: String,
    pub siren: Option<i32>,
    pub social_obj: Option<String>,
    pub requested_at: i64,
}

#[utoipa::path(
    get,
    path = "/api/admin/partners/pending",
    responses(
        (status = 200, description = "Partners waiting for admin validation", content_type = "application/json", body = [PendingPartner]),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[get("/admin/partners/pending")]
pub async fn get_pending_partners(db: web::Data<DatabaseConnection>) -> impl Responder {
    let pending_partners = partner::Entity::find()
        .join(sea_orm::JoinType::InnerJoin, partner::Relation::User.def())
        .join(sea_orm::JoinType::InnerJoin, user::Relation::State.def())
        .filter(state::Column::State.eq("waiting_activation"))
        .select_only()
        .column(partner::Column::Id)
        .column_as(user::Column::Name, "name")
        .column_as(user::Column::Mail, "mail")
        .column(partner::Column::Siren)
        .column(partner::Column::SocialObj)
        .column_as(state::Column::ModifiedAt, "requested_at")
        .into_model::<PendingPartner>()
        .all(db.get_ref())
        .await;

    match pending_partners {
        Ok(partners) => HttpResponse::Ok().json(partners),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[derive(Serialize, ToSchema, FromQueryResult)]
pub struct UserSummary {
    pub id: Uuid,
    pub name: String,
    pub mail: String,
    pub role: user::Role,
    pub state: Option<String>,
    pub created_at: i64,
}

#[utoipa::path(
    get,
    path = "/api/admin/users",
    responses(
        (status = 200, description = "All user accounts with their current state", content_type = "application/json", body = [UserSummary]),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[get("/admin/users")]
pub async fn get_users(db: web::Data<DatabaseConnection>) -> impl Responder {
    let users = user::Entity::find()
        .join(sea_orm::JoinType::LeftJoin, user::Relation::State.def())
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Name)
        .column(user::Column::Mail)
        .column(user::Column::Role)
        .column(user::Column::CreatedAt)
        .column_as(state::Column::State, "state")
        .into_model::<UserSummary>()
        .all(db.get_ref())
        .await;

    match users {
        Ok(users) => HttpResponse::Ok().json(users),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct PaymentRequest {
    pub qr_token: String,
    pub partner_id: Uuid,
    pub amount: f32,
}

#[utoipa::path(
    post,
    path = "/api/process/payment",
    request_body = PaymentRequest,
    responses(
        (status = 200, description = "Successfully processed payment", content_type = "application/json", body = transaction::Model),
        (status = 400, description = "Invalid request data", content_type = "application/json", body = ErrorResponse),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[post("/process/payment")]
pub async fn process_payment(
    req: HttpRequest,
    db: web::Data<DatabaseConnection>,
    body: web::Json<PaymentRequest>,
) -> impl Responder {
    let ip = crate::api::audit::client_ip(&req);
    let amount = body.amount;
    let partner_id = body.partner_id;

    if amount <= 0.0 {
        return HttpResponse::BadRequest().json(serde_json::json!({ "error": "Invalid amount" }));
    }

    let txn_result = db
        .transaction::<_, transaction::Model, sea_orm::DbErr>(|txn| {
            Box::pin(async move {
                let emp = employee::Entity::find()
                    .filter(employee::Column::QrToken.eq(&body.qr_token))
                    .one(txn)
                    .await?
                    .ok_or_else(|| {
                        sea_orm::DbErr::Custom("Employee token not found".to_string())
                    })?;

                let current_balance = emp.balance.unwrap_or(0.0);
                if current_balance < body.amount {
                    return Err(sea_orm::DbErr::Custom("Insufficient balance".to_string()));
                }

                let mut emp_active: employee::ActiveModel = emp.clone().into();
                emp_active.balance = ActiveValue::Set(Some(current_balance - body.amount));
                emp_active.update(txn).await?;

                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                let new_tx = transaction::ActiveModel {
                    id: ActiveValue::Set(Uuid::new_v4()),
                    timestamp: ActiveValue::Set(now),
                    success: ActiveValue::Set(true),
                    value: ActiveValue::Set(body.amount),
                    partner_id: ActiveValue::Set(body.partner_id),
                    employee_id: ActiveValue::Set(emp.id),
                };

                new_tx.insert(txn).await
            })
        })
        .await;

    match txn_result {
        Ok(tx) => {
            let entry = crate::api::audit::new_entry(
                Some(partner_id),
                Some("Partner".to_string()),
                "transaction_validated",
                Some("transaction".to_string()),
                Some(tx.id.to_string()),
                Some(serde_json::json!({
                    "amount": amount,
                    "employee_id": tx.employee_id,
                })),
                ip,
            );
            if let Err(e) = crate::api::audit::log_audit(db.get_ref(), entry).await {
                log::error!("audit log failed: {e}");
            }
            HttpResponse::Ok().json(tx)
        }
        Err(sea_orm::TransactionError::Transaction(msg)) => {
            let entry = crate::api::audit::new_entry(
                Some(partner_id),
                Some("Partner".to_string()),
                "transaction_refused",
                Some("payment_attempt".to_string()),
                None,
                Some(serde_json::json!({
                    "amount": amount,
                    "reason": msg.to_string(),
                })),
                ip,
            );
            if let Err(e) = crate::api::audit::log_audit(db.get_ref(), entry).await {
                log::error!("audit log failed: {e}");
            }
            HttpResponse::BadRequest().json(serde_json::json!({ "error": msg.to_string() }))
        }
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct TokenBody {
    pub qr_token: String,
}

#[utoipa::path(
    post,
    path = "/api/begin/payment",
    params(
        ("Authorization" = String, Header, description = "Bearer token for authentication")
    ),
    responses(
        (status = 200, description = "Successfully generated qr token", content_type = "application/json", body = TokenBody),
        (status = 400, description = "Invalid request data", content_type = "application/json", body = ErrorResponse),
        (status = 401, description = "Unauthorized", content_type = "application/json", body = ErrorResponse),
        (status = 404, description = "User not found", content_type = "application/json", body = ErrorResponse),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[post("/begin/payment")]
pub async fn begin_payment(req: HttpRequest, db: web::Data<DatabaseConnection>) -> impl Responder {
    let Some(auth) = req.headers().get("Authorization") else {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({ "error": "Missing Authorization header" }));
    };

    let Ok(auth) = auth.to_str() else {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({ "error": "Invalid Authorization header" }));
    };

    let Some(token) = auth.strip_prefix("Bearer ") else {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({ "error": "Invalid token format" }));
    };

    let Ok(uuid) = uuid::Uuid::parse_str(token) else {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "error": "Invalid UUID format" }));
    };

    let query = user::Entity::find().filter(user::Column::Id.eq(uuid));

    match get_one(db.get_ref(), query).await {
        Ok(Some(u)) => {
            let mut employee: employee::ActiveModel =
                match get_by_id::<employee::Entity, _>(db.get_ref(), u.id).await {
                    Ok(Some(emp)) => employee::ActiveModel::from(emp),
                    Ok(None) => {
                        return HttpResponse::NotFound()
                            .json(serde_json::json!({ "error": "Employee record not found" }));
                    }
                    Err(e) => {
                        return HttpResponse::InternalServerError()
                            .json(serde_json::json!({ "error": e.to_string() }));
                    }
                };
            let token_body = TokenBody {
                qr_token: Uuid::new_v4().to_string(),
            };
            employee.qr_token = ActiveValue::Set(Some(token_body.qr_token.clone()));
            employee.qr_token_created_at = ActiveValue::Set(Some(chrono::Utc::now().timestamp()));
            match update::<employee::Entity, _>(db.get_ref(), employee).await {
                Ok(_) => HttpResponse::Ok().json(token_body),
                Err(e) => HttpResponse::InternalServerError()
                    .json(serde_json::json!({ "error": e.to_string() })),
            }
        }
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({ "error": "User not found" })),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[derive(Serialize, ToSchema, FromQueryResult)]
pub struct EmployeeTransaction {
    pub id: Uuid,
    pub timestamp: i64,
    pub success: bool,
    pub value: f32,
    pub partner_id: Uuid,
    pub employee_id: Uuid,
    pub partner_name: String,
}

#[utoipa::path(
    get,
    path = "/api/employees/{id}/transactions",
    responses(
        (status = 200, description = "Successfully retrieved transactions", content_type = "application/json", body = [EmployeeTransaction]),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[get("/employees/{id}/transactions")]
pub async fn get_employee_transactions(
    db: web::Data<DatabaseConnection>,
    id: web::Path<Uuid>,
) -> impl Responder {
    let res = transaction::Entity::find()
        .join(
            sea_orm::JoinType::InnerJoin,
            transaction::Relation::Partner.def(),
        )
        .join(sea_orm::JoinType::InnerJoin, partner::Relation::User.def())
        .filter(transaction::Column::EmployeeId.eq(id.into_inner()))
        .select_only()
        .column(transaction::Column::Id)
        .column(transaction::Column::Timestamp)
        .column(transaction::Column::Success)
        .column(transaction::Column::Value)
        .column(transaction::Column::PartnerId)
        .column(transaction::Column::EmployeeId)
        .column_as(user::Column::Name, "partner_name")
        .into_model::<EmployeeTransaction>()
        .all(db.get_ref())
        .await;

    match res {
        Ok(txs) => HttpResponse::Ok().json(txs),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/partners/{id}/transactions",
    responses(
        (status = 200, description = "Successfully retrieved transactions", content_type = "application/json", body = [transaction::Model]),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[get("/partners/{id}/transactions")]
pub async fn get_partner_transactions(
    db: web::Data<DatabaseConnection>,
    id: web::Path<Uuid>,
) -> impl Responder {
    let res = transaction::Entity::find()
        .filter(transaction::Column::PartnerId.eq(id.into_inner()))
        .all(db.get_ref())
        .await;

    match res {
        Ok(txs) => HttpResponse::Ok().json(txs),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}

#[derive(Serialize, ToSchema, FromQueryResult)]
pub struct PartnerDirectoryEntry {
    pub id: Uuid,
    pub social_obj: Option<String>,
    pub category: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/directory",
    responses(
        (status = 200, description = "Active partners referenced by the ministry", content_type = "application/json", body = [PartnerDirectoryEntry]),
        (status = 500, description = "Internal server error", content_type = "application/json", body = ErrorResponse)
    )
)]
#[get("/directory")]
pub async fn get_partner_directory(db: web::Data<DatabaseConnection>) -> impl Responder {
    let partners = partner::Entity::find()
        .join(sea_orm::JoinType::InnerJoin, partner::Relation::User.def())
        .join(sea_orm::JoinType::InnerJoin, user::Relation::State.def())
        .filter(state::Column::State.eq("active"))
        .select_only()
        .column(partner::Column::Id)
        .column(partner::Column::SocialObj)
        .column(partner::Column::Category)
        .into_model::<PartnerDirectoryEntry>()
        .all(db.get_ref())
        .await;

    match partners {
        Ok(partners) => HttpResponse::Ok().json(partners),
        Err(e) => {
            HttpResponse::InternalServerError().json(serde_json::json!({ "error": e.to_string() }))
        }
    }
}
