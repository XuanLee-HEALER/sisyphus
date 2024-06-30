use chrono::{DateTime, Local};

struct Resource {
    id: u64,
    name: String,
    description: String,
    resource_type: ResourceType,
    resource_form: ResourceForm,
    level: u8,
    contains: Option<Vec<Resource>>,
    status: ResourceStatus,
    create_datetime: DateTime<Local>,
    last_update_datetime: DateTime<Local>,
    deleted: bool,
    delete_datetime: Option<DateTime<Local>>,
}

enum ResourceStatus {
    CREATED,
    DEPLOYED,
    PREPARED,
    USING,
    EXCEPTION,
    REVOKING,
    UNAVAILABLE,
    DELETED,
}

enum ResourceType {
    OS_TYPE,
    DB_TYPE,
    APP_TYPE,
    PROFILER_TYPE,
}

enum ResourceForm {
    Single,
    Composed,
}

struct Scene {
    resources: Option<Vec<Resource>>,
}

fn main() {
    println!("Hello, world!");
}
