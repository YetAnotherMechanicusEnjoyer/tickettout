CREATE TABLE users (
  id UUID PRIMARY KEY,
  mail VARCHAR(255) NOT NULL UNIQUE,
  name VARCHAR(255) NOT NULL,
  password TEXT NOT NULL,
  role VARCHAR(32) NOT NULL,
  created_at BIGINT NOT NULL
);

CREATE TABLE state (
  id UUID PRIMARY KEY REFERENCES users(id),
  state VARCHAR(32) NOT NULL,
  reason VARCHAR(255),
  modified_at BIGINT NOT NULL
);

CREATE TABLE employee (
  id UUID PRIMARY KEY REFERENCES users(id),
  balance REAL,
  qr_token VARCHAR(255),
  qr_token_created_at BIGINT
);

CREATE TABLE partner (
  id UUID PRIMARY KEY REFERENCES users(id),
  coordinate POINT,
  siren INT4 UNIQUE,
  social_obj VARCHAR(255),
  verification BOOL,
  category VARCHAR(255)
);

CREATE TABLE admin (
  id UUID PRIMARY KEY REFERENCES users(id)
);

CREATE TABLE transaction (
  id UUID PRIMARY KEY,
  timestamp BIGINT NOT NULL,
  success BOOL,
  value REAL,
  partner_id UUID REFERENCES partner(id),
  employee_id UUID REFERENCES employee(id)
);

CREATE TABLE audit (
    id UUID PRIMARY KEY,
    occurred_at BIGINT NOT NULL,
    actor_id UUID,
    actor_role VARCHAR(50),
    action VARCHAR(100) NOT NULL,
    target_type VARCHAR(50),
    target_id VARCHAR(255),
    payload JSONB,
    ip VARCHAR(45),
    previous_hash VARCHAR(64) NOT NULL
);

CREATE ROLE cartepro_app LOGIN PASSWORD 'cartepro_dev_password';
GRANT USAGE ON SCHEMA public TO cartepro_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON
  users, state, employee, partner, admin, transaction
  TO cartepro_app;

GRANT SELECT, INSERT ON audit TO cartepro_app;
REVOKE UPDATE, DELETE ON audit FROM cartepro_app;
