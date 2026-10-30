use std::{env, error::Error};

#[derive(Clone, Debug)]
pub struct McpConfig {
    pub host: String,
    pub port: u16,
}

impl McpConfig {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let host = env::var("MCP_HOST").unwrap_or_else(|_| "0.0.0.0".to_owned());
        let port = env::var("MCP_PORT")
            .unwrap_or_else(|_| "8001".to_owned())
            .parse::<u16>()?;

        Ok(Self { host, port })
    }
}

#[derive(Clone, Debug)]
pub struct DataConfig {
    pub database_url: String,
    pub max_connections: u32,
}

impl DataConfig {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data/demo.db".to_owned());
        let max_connections = env::var("MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10".to_owned())
            .parse::<u32>()?;
        Ok(Self {
            database_url,
            max_connections,
        })
    }
}

#[derive(Clone)]
pub struct AuthConfig {
    pub tenant_id: String,
    pub audience: String,
    pub resource: reqwest::Url,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let tenant_id = env::var("ENTRA_TENANT_ID")?;
        let audience = env::var("ENTRA_API_CLIENT_ID")?;
        let resource = reqwest::Url::parse(&env::var("MCP_RESOURCE")?)?;
        Self::new(tenant_id, audience, resource)
    }

    fn new(
        tenant_id: String,
        audience: String,
        resource: reqwest::Url,
    ) -> Result<Self, Box<dyn Error>> {
        if tenant_id.trim().is_empty() || audience.trim().is_empty() {
            return Err("Entra tenant and API application IDs must not be empty".into());
        }
        let loopback = resource.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
        if !(resource.scheme() == "https" || resource.scheme() == "http" && loopback)
            || resource.path() != "/mcp"
            || resource.query().is_some()
            || resource.fragment().is_some()
            || !resource.username().is_empty()
            || resource.password().is_some()
        {
            return Err(
                "MCP_RESOURCE must end in /mcp and use HTTPS, or HTTP on a loopback host".into(),
            );
        }
        Ok(Self {
            tenant_id,
            audience,
            resource,
        })
    }

    pub fn issuer(&self) -> String {
        format!("https://login.microsoftonline.com/{}/v2.0", self.tenant_id)
    }

    pub fn scope(&self) -> String {
        format!("{}/access_as_user", self.resource)
    }

    pub fn metadata_url(&self) -> String {
        let mut url = self.resource.clone();
        url.set_path("/.well-known/oauth-protected-resource/mcp");
        url.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_allows_https_and_loopback_http_only() {
        for (resource, valid) in [
            ("https://example.com/mcp", true),
            ("http://127.0.0.1:8000/mcp", true),
            ("http://localhost:8000/mcp", true),
            ("http://[::1]:8000/mcp", true),
            ("http://0.0.0.0:8000/mcp", false),
            ("http://192.168.1.1:8000/mcp", false),
            ("http://example.com/mcp", false),
            ("http://localhost.example.com/mcp", false),
            ("https://example.com/mcp/", false),
            ("https://example.com/mcp?query=true", false),
            ("https://example.com/mcp#fragment", false),
            ("https://user:password@example.com/mcp", false),
        ] {
            let config = AuthConfig::new(
                "demo-tenant".into(),
                "demo-api".into(),
                resource.parse().unwrap(),
            );
            assert_eq!(config.is_ok(), valid, "{resource}");
        }
    }
}
