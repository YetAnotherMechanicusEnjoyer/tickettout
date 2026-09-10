use crate::models::Role;
use actix_web::web;
use utoipa::OpenApi;

pub mod audit;
mod auth;
mod buisness;
mod crud;
mod csv;
mod docs;
mod echo;
mod health;
mod resources;
mod user;
mod users;

use crate::entities::{admin, employee, partner, state};
use resources::*;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .service(health::health)
            .service(echo::echo)
            .service(csv::transactions_to_csv)
            .configure(user::configure)
            .service(audit::export_audit)
            .service(audit::list_audit)
            .service(buisness::get_partner_directory)
            .service(buisness::get_pending_partners)
            .service(buisness::get_users)
            .service(buisness::process_payment)
            .service(buisness::begin_payment)
            .service(buisness::get_employee_transactions)
            .service(buisness::get_partner_transactions)
            .configure(auth::configure)
            .configure(docs::configure)
            .service(crud::crud_scope::<employee::Entity, EmployeeAdapter>(
                "/employees",
            ))
            .service(crud::crud_scope::<partner::Entity, PartnerAdapter>(
                "/partners",
            ))
            .service(crud::crud_scope::<state::Entity, StateAdapter>("/states"))
            .service(crud::crud_scope::<admin::Entity, AdminAdapter>("/admins")),
    );
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health::health,
        echo::echo,
        csv::transactions_to_csv,
        user::get,
        user::pass,
        user::put,
        user::delete,
        buisness::get_pending_partners,
        buisness::process_payment,
        buisness::begin_payment,
        buisness::get_employee_transactions,
        buisness::get_partner_transactions,
        buisness::get_users,
        auth::login,
        auth::register,
    ),
    components(
        schemas(
            Role,
            user::GetResponse, user::PassRequest, user::PutRequest,
            // partner::Model, employee::Model, state::Model, admin::Model,
            buisness::PaymentRequest, // transaction::Model,
            auth::LoginRequest, auth::RegisterRequest, auth::AuthResponse,
            buisness::PendingPartner, buisness::UserSummary, buisness::EmployeeTransaction,
        )
    )
)]
pub struct ApiDoc;
