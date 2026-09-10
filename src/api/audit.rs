use crate::{
    db::get_one,
    entities::user::{self as User, Role},
};
use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sha::sha256;
use openssl::sign::Signer;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use std::env;

async fn require_admin(
    req: &actix_web::HttpRequest,
    db: &DatabaseConnection,
) -> Result<crate::entities::user::Model, HttpResponse> {
    let auth = req
        .headers()
        .get("Authorization")
        .ok_or_else(HttpResponse::Unauthorized)?;
    let auth = auth.to_str().map_err(|_| HttpResponse::Unauthorized())?;
    let token = auth
        .strip_prefix("Bearer ")
        .ok_or_else(HttpResponse::Unauthorized)?;
    let uuid = uuid::Uuid::parse_str(token).map_err(|_| HttpResponse::BadRequest())?;

    let query = User::Entity::find().filter(User::Column::Id.eq(uuid));
    match get_one(db, query).await {
        Ok(Some(u)) if u.role == Role::Admin => Ok(u),
        Ok(Some(_)) => Err(HttpResponse::Forbidden().finish()),
        Ok(None) => Err(HttpResponse::NotFound().finish()),
        Err(e) => Err(HttpResponse::InternalServerError().body(e.to_string())),
    }
}

#[derive(Deserialize)]
pub struct AuditQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub actor_id: Option<uuid::Uuid>,
    pub action: Option<String>,
    pub page: Option<u64>,
    pub per_page: Option<u64>,
}

#[derive(Serialize)]
pub struct AuditPage {
    pub items: Vec<crate::entities::audit::Model>,
    pub page: u64,
    pub per_page: u64,
    pub total_items: u64,
    pub total_pages: u64,
}

#[get("/v1/admin/audit")]
pub async fn list_audit(
    req: actix_web::HttpRequest,
    query: web::Query<AuditQuery>,
    db: web::Data<DatabaseConnection>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req, db.get_ref()).await {
        return resp;
    }

    let mut q = crate::entities::audit::Entity::find();
    if let Some(from) = query.from {
        q = q.filter(crate::entities::audit::Column::OccurredAt.gte(from));
    }
    if let Some(to) = query.to {
        q = q.filter(crate::entities::audit::Column::OccurredAt.lte(to));
    }
    if let Some(actor_id) = query.actor_id {
        q = q.filter(crate::entities::audit::Column::ActorId.eq(actor_id));
    }
    if let Some(action) = &query.action {
        q = q.filter(crate::entities::audit::Column::Action.eq(action.clone()));
    }
    q = q.order_by_desc(crate::entities::audit::Column::OccurredAt);

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 200);

    let paginator = q.paginate(db.get_ref(), per_page);
    let total_items = match paginator.num_items().await {
        Ok(n) => n,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };
    let total_pages = paginator.num_pages().await.unwrap_or(0);
    let items = match paginator.fetch_page(page - 1).await {
        Ok(items) => items,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    HttpResponse::Ok().json(AuditPage {
        items,
        page,
        per_page,
        total_items,
        total_pages,
    })
}

pub fn compute_hash(record: &crate::entities::audit::Model) -> String {
    let payload_str = record
        .payload
        .as_ref()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let raw = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        record.id,
        record.occurred_at,
        record.actor_id.map(|v| v.to_string()).unwrap_or_default(),
        record.actor_role.as_deref().unwrap_or_default(),
        record.action,
        record.target_type.as_deref().unwrap_or_default(),
        record.target_id.as_deref().unwrap_or_default(),
        payload_str,
        record.ip.as_deref().unwrap_or_default(),
        record.previous_hash
    );
    let digest = sha256(raw.as_bytes());
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn new_entry(
    actor_id: Option<uuid::Uuid>,
    actor_role: Option<String>,
    action: &str,
    target_type: Option<String>,
    target_id: Option<String>,
    payload: Option<serde_json::Value>,
    ip: Option<String>,
) -> crate::entities::audit::ActiveModel {
    crate::entities::audit::ActiveModel {
        id: ActiveValue::Set(uuid::Uuid::new_v4()),
        occurred_at: ActiveValue::Set(chrono::Utc::now().timestamp_millis()),
        actor_id: ActiveValue::Set(actor_id),
        actor_role: ActiveValue::Set(actor_role),
        action: ActiveValue::Set(action.to_string()),
        target_type: ActiveValue::Set(target_type),
        target_id: ActiveValue::Set(target_id),
        payload: ActiveValue::Set(payload),
        ip: ActiveValue::Set(ip),
        previous_hash: ActiveValue::Set(String::new()),
    }
}

pub fn client_ip(req: &actix_web::HttpRequest) -> Option<String> {
    req.peer_addr().map(|addr| addr.ip().to_string())
}

pub async fn log_audit(
    db: &DatabaseConnection,
    mut new_log: crate::entities::audit::ActiveModel,
) -> Result<(), sea_orm::DbErr> {
    let last_record = crate::entities::audit::Entity::find()
        .order_by_desc(crate::entities::audit::Column::OccurredAt)
        .one(db)
        .await?;

    let prev_hash = match last_record {
        Some(record) => compute_hash(&record),
        None => "GENESIS".to_string(),
    };

    new_log.previous_hash = ActiveValue::Set(prev_hash);
    new_log.insert(db).await?;
    Ok(())
}

#[derive(Serialize)]
pub struct AuditExport {
    pub data: Vec<crate::entities::audit::Model>,
    pub data_json: String,
    pub signature: String,
}

#[derive(Deserialize)]
pub struct AuditExportQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[get("/v1/admin/audit/export")]
pub async fn export_audit(
    req: HttpRequest,
    query: web::Query<AuditExportQuery>,
    db: web::Data<DatabaseConnection>,
) -> impl Responder {
    if let Err(resp) = require_admin(&req, db.get_ref()).await {
        return resp;
    }

    let mut q = crate::entities::audit::Entity::find();
    if let Some(from) = query.from {
        q = q.filter(crate::entities::audit::Column::OccurredAt.gte(from));
    }
    if let Some(to) = query.to {
        q = q.filter(crate::entities::audit::Column::OccurredAt.lte(to));
    }
    let logs = match q
        .order_by_asc(crate::entities::audit::Column::OccurredAt)
        .all(db.get_ref())
        .await
    {
        Ok(l) => l,
        Err(e) => return HttpResponse::InternalServerError().body(e.to_string()),
    };

    let json_data = serde_json::to_string(&logs).unwrap_or_default();

    let secret_key = env::var("AUDIT_EXPORT_KEY")
        .or_else(|_| env::var("SECRET"))
        .unwrap_or("default_unsafe_key".into());
    let pkey = PKey::hmac(secret_key.as_bytes()).unwrap();
    let mut signer = Signer::new(MessageDigest::sha256(), &pkey).unwrap();
    signer.update(json_data.as_bytes()).unwrap();

    let signature = signer
        .sign_to_vec()
        .unwrap()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    HttpResponse::Ok().json(AuditExport {
        data: logs,
        data_json: json_data,
        signature,
    })
}
