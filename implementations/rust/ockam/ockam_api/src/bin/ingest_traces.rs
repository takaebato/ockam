use aws_sdk_s3::Client;
use flate2::read::GzDecoder;
use itertools::Itertools;
use ockam_api::ApiError;
use ockam_core::compat::collections::HashMap;
use serde::de::{Error, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fmt, fs};
use time::{format_description, OffsetDateTime};

#[tokio::main]
async fn main() -> ockam_api::Result<()> {
    // let spans =
    //     read_spans_from_directory("/Users/etorreborre/projects/ockam/ockam-5/traces").await?;
    let spans = read_spans_from_s3(
        "prod",
        "ockam-opentelemetry-collector-prod",
        "events/year=2024/month=10/day=30/hour=09/minute=54",
    )
    .await?;
    save_spans(&spans).await?;
    process_spans(spans).await?;
    Ok(())
}

async fn read_spans_from_s3(
    sso_profile: &str,
    bucket: &str,
    prefix: &str,
) -> ockam_api::Result<Vec<Span>> {
    let config = aws_config::from_env()
        .profile_name(sso_profile)
        .region("us-west-1")
        .load()
        .await;
    let client = Client::new(&config);
    let objects = client
        .list_objects_v2()
        .bucket(bucket)
        .prefix(prefix)
        .send()
        .await
        .into_api_error()?;

    let mut result = vec![];
    for object in objects.contents.unwrap_or(vec![]) {
        let key = object.key.unwrap_or("n/a".to_string());
        if key.contains("traces") {
            println!("getting the object {key}");
            let object = client
                .get_object()
                .bucket(bucket)
                .key(key)
                .send()
                .await
                .map_err(|e| ApiError::General(format!("{e}")))?;
            let bytes = object.body.collect().await.into_api_error()?.to_vec();
            result.extend(extract(&bytes)?);
        }
    }

    Ok(result)
}

async fn read_spans_from_directory(dir_name: &str) -> ockam_api::Result<Vec<Span>> {
    let mut result = vec![];
    let mut files = vec![];
    list_files_with_pattern(Path::new(dir_name), "traces", &mut files)?;
    for file in files {
        // println!("Reading file: {:?}", file);
        let mut file = File::open(&file)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        result.extend(extract(&bytes.as_slice())?);
    }
    Ok(result)
}

fn list_files_with_pattern(
    dir: &Path,
    pattern: &str,
    files: &mut Vec<PathBuf>,
) -> ockam_api::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Recurse into the directory
            list_files_with_pattern(&path, pattern, files)?;
        } else if path.is_file()
            && path
                .file_name()
                .map(|name| name.to_string_lossy().contains(pattern))
                .unwrap_or(false)
        {
            // If it's a file and matches the pattern, add to list
            files.push(path);
        }
    }
    files.sort();
    Ok(())
}

fn extract(bytes: &[u8]) -> ockam_api::Result<Vec<Span>> {
    let decompressed = decompress_gzip(bytes)?;
    let as_string = String::from_utf8(decompressed).into_api_error()?;
    let resource_spans = deserialize(&as_string)?;
    Ok(resource_spans.spans)
}

async fn save_spans(_spans: &Vec<Span>) -> ockam_api::Result<()> {
    Ok(())
}

async fn process_spans(spans: Vec<Span>) -> ockam_api::Result<()> {
    let mut user_events_by_trace_id: HashMap<String, Vec<UserEvent>> = HashMap::new();
    for span in spans {
        if let Some(user_event) = span.to_user_event() {
            match user_events_by_trace_id.get_mut(&span.trace_id) {
                Some(events) => events.push(user_event),
                None => {
                    user_events_by_trace_id.insert(span.trace_id, vec![user_event]);
                    ()
                }
            }
        }
    }

    for (trace_id, user_events) in user_events_by_trace_id.iter_mut() {
        println!("User journey {trace_id}:\n");
        match find_summary_event(user_events) {
            Some(event) => println!("{}", &event.summary()),
            None => (),
        }
        user_events.sort_by_key(|e| e.start_time);
        println!(
            "{}",
            user_events
                .iter()
                .map(|u| format!("{}", &u.details()))
                .collect::<Vec<String>>()
                .join("\n")
        );
        println!("\n");
    }

    Ok(())
}

fn find_summary_event(events: &Vec<UserEvent>) -> Option<&UserEvent> {
    events.iter().find_or_first(|e| e.user_email.is_some())
}

fn deserialize(text: &str) -> ockam_api::Result<ResourceSpans> {
    serde_json::from_str(text).into_api_error()
}

fn decompress_gzip(compressed_data: &[u8]) -> ockam_api::Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(compressed_data);
    let mut decompressed_data = Vec::new();
    decoder.read_to_end(&mut decompressed_data)?;
    Ok(decompressed_data)
}

#[derive(Debug, Eq, PartialEq)]
struct ResourceSpans {
    attributes: HashMap<String, String>,
    spans: Vec<Span>,
}

#[derive(Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Span {
    #[serde(default)]
    scope: String,
    trace_id: String,
    span_id: String,
    #[serde(deserialize_with = "deserialize_parent_span_id")]
    parent_span_id: Option<String>,
    name: String,
    #[serde(deserialize_with = "deserialize_time", rename = "startTimeUnixNano")]
    start_time: OffsetDateTime,
    #[serde(deserialize_with = "deserialize_time", rename = "endTimeUnixNano")]
    end_time: OffsetDateTime,
    #[serde(deserialize_with = "deserialize_attributes")]
    attributes: HashMap<String, String>,
    #[serde(default)]
    links: Vec<Link>,
}

impl Span {
    fn to_user_event(&self) -> Option<UserEvent> {
        match self.attributes.get("app.event.ockam_home") {
            Some(ockam_home) => {
                let execution_trace_id = self.links.first().map(|link| link.trace_id.clone());
                let command = UserEvent {
                    name: self.name.clone(),
                    ockam_home: ockam_home.clone(),
                    command: self.attributes.get("app.event.command").cloned(),
                    error_message: self.attributes.get("app.event.error_message").cloned(),
                    start_time: self.start_time,
                    end_time: self.end_time,
                    node_name: self.attributes.get("app.event.node_name").cloned(),
                    ockam_version: self.attributes.get("app.event.ockam_version").cloned(),
                    ockam_git_hash: self.attributes.get("app.event.ockam_git_hash").cloned(),
                    space_name: self.attributes.get("app.event.space.name").cloned(),
                    space_id: self.attributes.get("app.event.space.id").cloned(),
                    project_name: self.attributes.get("app.event.project.name").cloned(),
                    project_id: self.attributes.get("app.event.project.id").cloned(),
                    project_access_route: self
                        .attributes
                        .get("app.event.project_access_route")
                        .cloned(),
                    user_name: self.attributes.get("app.user_name").cloned(),
                    user_email: self.attributes.get("app.user_email").cloned(),
                    ockam_developer: self.attributes.get("app.event.ockam_developer").cloned(),
                    execution_trace_id,
                };
                Some(command)
            }
            None => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct UserEvent {
    name: String,
    ockam_home: String,
    command: Option<String>,
    error_message: Option<String>,
    start_time: OffsetDateTime,
    end_time: OffsetDateTime,
    node_name: Option<String>,
    ockam_version: Option<String>,
    ockam_git_hash: Option<String>,
    space_name: Option<String>,
    space_id: Option<String>,
    project_name: Option<String>,
    project_id: Option<String>,
    project_access_route: Option<String>,
    user_name: Option<String>,
    user_email: Option<String>,
    ockam_developer: Option<String>,
    execution_trace_id: Option<String>,
}

impl UserEvent {
    fn summary(&self) -> UserEventSummary {
        UserEventSummary {
            ockam_home: self.ockam_home.clone(),
            ockam_version: self.ockam_version.clone(),
            ockam_git_hash: self.ockam_git_hash.clone(),
            space_name: self.space_name.clone(),
            space_id: self.space_id.clone(),
            project_name: self.project_name.clone(),
            project_id: self.project_id.clone(),
            project_access_route: self.project_access_route.clone(),
            user_name: self.user_name.clone(),
            user_email: self.user_email.clone(),
            ockam_developer: self.ockam_developer.clone(),
        }
    }

    fn details(&self) -> UserEventDetails {
        UserEventDetails {
            name: self.name.clone(),
            command: self.command.clone(),
            error_message: self.error_message.clone(),
            start_time: self.start_time.clone(),
            node_name: self.node_name.clone(),
            execution_trace_id: self.execution_trace_id.clone(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct UserEventSummary {
    ockam_home: String,
    ockam_version: Option<String>,
    ockam_git_hash: Option<String>,
    space_name: Option<String>,
    space_id: Option<String>,
    project_name: Option<String>,
    project_id: Option<String>,
    project_access_route: Option<String>,
    user_name: Option<String>,
    user_email: Option<String>,
    ockam_developer: Option<String>,
}

impl Display for UserEventSummary {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(v) = &self.user_name {
            writeln!(f, "User name:\t{v}")?
        };
        if let Some(v) = &self.user_email {
            writeln!(f, "User email:\t{v}")?
        };
        writeln!(f, "Ockam home:\t{}", self.ockam_home)?;
        if let Some(v) = &self.ockam_version {
            writeln!(f, "Ockam version:\t{v}")?
        };
        if let Some(v) = &self.ockam_git_hash {
            writeln!(f, "Ockam git hash:\t{v}")?
        };
        if let Some(v) = &self.ockam_developer {
            writeln!(f, "Ockam dev.:\t{v}")?
        };
        if let Some(v) = &self.space_name {
            writeln!(f, "Space name:\t{v}")?
        };
        if let Some(v) = &self.space_id {
            writeln!(f, "Space id:\t{v}")?
        };
        if let Some(v) = &self.project_name {
            writeln!(f, "Project name:\t{v}")?
        };
        if let Some(v) = &self.project_id {
            writeln!(f, "Project id:\t{v}")?
        };
        if let Some(v) = &self.project_access_route {
            writeln!(f, "Project access route:\t{v}")?
        };
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
struct UserEventDetails {
    name: String,
    command: Option<String>,
    error_message: Option<String>,
    start_time: OffsetDateTime,
    node_name: Option<String>,
    execution_trace_id: Option<String>,
}

impl Display for UserEventDetails {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let date_format =
            format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();
        write!(f, "[{}] ", self.start_time.format(&date_format).unwrap())?;
        writeln!(f, "{}", self.name)?;
        if let Some(v) = &self.command {
            writeln!(f, "Command: {v}")?
        };
        if let Some(v) = &self.error_message {
            writeln!(f, "Error: {v}")?
        };
        if let Some(v) = &self.node_name {
            writeln!(f, "Node: {v}")?
        };
        if let Some(v) = &self.execution_trace_id {
            writeln!(f, "Execution trace id: {v}")?
        };
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ResourceSpans {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ResourceSpanVisitor)
    }
}

struct ResourceSpanVisitor;

impl<'de> Visitor<'de> for ResourceSpanVisitor {
    type Value = ResourceSpans;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an object with `resource` and `scopeSpans` fields fields")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if let Some((Value::String(key), Value::Array(values))) =
            map.next_entry::<Value, Value>()?
        {
            if key == "resourceSpans" {
                let mut resource_attributes = Attributes(HashMap::new());
                let mut resource_spans: Spans = Spans(vec![]);
                for value in values {
                    if let (Some(resource), Some(spans)) =
                        (value.get("resource"), value.get("scopeSpans"))
                    {
                        if let Some(attributes) = resource.get("attributes") {
                            let deserialized_attributes: Attributes = serde_json::from_value(attributes.clone())
                                .map_err(|e| {
                                    Error::custom(format!(
                                        "cannot deserialize as a list of resource attributes {e:?}. Attributes are {}", attributes.clone()
                                    ))
                                })?;
                            resource_attributes.0.extend(deserialized_attributes.0);
                        };
                        let deserialized_spans: Spans = serde_json::from_value(spans.clone())
                            .map_err(|e| {
                                Error::custom(format!(
                                    "cannot deserialize as an array of spans {e:?}. Spans are {}",
                                    spans.clone()
                                ))
                            })?;
                        resource_spans.0.extend(deserialized_spans.0);
                    }
                }
                return Ok(ResourceSpans {
                    attributes: resource_attributes.0,
                    spans: resource_spans.0,
                });
            }
        };
        Err(Error::custom(
            "cannot deserialize as an array of spans, no `resourceSpans` field found",
        ))
    }
}

fn deserialize_attributes<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    Attributes::deserialize(deserializer).map(|a| a.0)
}

fn deserialize_time<'de, D>(deserializer: D) -> Result<OffsetDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let as_string = String::deserialize(deserializer)?;
    match i128::from_str(&as_string) {
        Ok(t) => match OffsetDateTime::from_unix_timestamp_nanos(t) {
            Ok(dt) => Ok(dt),
            Err(e) => Err(D::Error::custom(format!(
                "cannot deserialize as an OffsetDateTime {e:?}"
            ))),
        },
        Err(e) => Err(D::Error::custom(format!(
            "cannot deserialize as an OffsetDateTime {e:?}"
        ))),
    }
}

fn deserialize_parent_span_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Attributes(HashMap<String, String>);

impl<'de> Deserialize<'de> for Attributes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(AttributesVisitor)
    }
}

struct AttributesVisitor;

impl<'de> Visitor<'de> for AttributesVisitor {
    type Value = Attributes;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an array of objects with `key` and `value` fields")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut map = HashMap::new();
        while let Some(Value::Object(attributes)) = seq.next_element::<Value>()? {
            if let (Some(Value::String(key)), Some(Value::Object(value))) =
                (attributes.get("key"), attributes.get("value"))
            {
                if let Some(Value::String(v)) = value.get("stringValue") {
                    map.insert(key.to_string(), v.to_string());
                }
                if let Some(Value::Bool(v)) = value.get("boolValue") {
                    map.insert(key.to_string(), v.to_string());
                }
                if let Some(Value::Number(v)) = value.get("intValue") {
                    map.insert(key.to_string(), v.to_string());
                }
            }
        }

        Ok(Attributes(map))
    }
}

struct Spans(Vec<Span>);

impl<'de> Deserialize<'de> for Spans {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(SpansVisitor)
    }
}

struct SpansVisitor;

impl<'de> Visitor<'de> for SpansVisitor {
    type Value = Spans;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an array of spans")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut result = vec![];
        while let Some(Value::Object(attributes)) = seq.next_element::<Value>()? {
            if let (Some(Value::Object(scope)), Some(Value::Array(scope_spans))) =
                (attributes.get("scope"), attributes.get("spans"))
            {
                let mut spans: Vec<Span> =
                    serde_json::from_value(Value::Array(scope_spans.clone())).map_err(|e| {
                        self::Error::custom(format!(
                            "cannot deserialize as an array of spans {e:?}"
                        ))
                    })?;
                if let Some(Value::String(scope_name)) = scope.get("name") {
                    for span in &mut spans {
                        span.scope = scope_name.to_string();
                    }
                    result.extend(spans)
                }
            }
        }

        Ok(Spans(result))
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    trace_id: String,
    span_id: String,
}

#[cfg(test)]
mod test {
    use super::*;
    use ockam_core::compat::collections::HashMap;

    #[test]
    fn test_link_deserialization() {
        let serialized = r#"{"traceId":"3c48a8e0e4bd3c4b54c908c8329fdb51","spanId":"17168adfd2fc1679","flags":1}"#;
        let expected = Link {
            trace_id: "3c48a8e0e4bd3c4b54c908c8329fdb51".into(),
            span_id: "17168adfd2fc1679".into(),
        };
        match serde_json::from_str::<Link>(serialized) {
            Ok(deserialized) => assert_eq!(deserialized, expected),
            Err(e) => assert!(false, "deserialization error: {}", e),
        };
    }

    #[test]
    fn test_attributes_deserialization() {
        let serialized = r#"[
           {"key":"service","value":{"stringValue":"ockam"}},
           {"key":"developer","value":{"boolValue":true}},
           {"key":"count","value":{"intValue":10}}
        ]"#;
        let mut map = HashMap::new();
        map.insert("service".to_string(), "ockam".to_string());
        map.insert("developer".to_string(), "true".to_string());
        map.insert("count".to_string(), "10".to_string());
        let expected = Attributes(map);
        match serde_json::from_str::<Attributes>(serialized) {
            Ok(deserialized) => assert_eq!(deserialized, expected),
            Err(e) => assert!(false, "deserialization error: {}", e),
        };
    }

    #[test]
    fn test_resource_spans_deserialization() {
        let serialized = r#"
        { "resourceSpans" : [
            {
              "resource":{
                "attributes":[
                  {"key":"service","value":{"stringValue":"ockam"}},
                  {"key":"host","value":{"stringValue":"rendezvous"}}
                ]
              },
              "scopeSpans":[{
                "scope":{"name":"ockam"},
                "spans":[{
                  "traceId":"031dc45a859a886ca58e2b1befe31ad0",
                  "spanId":"b9276e9d706ad874",
                  "parentSpanId":"",
                  "flags":1,
                  "name":"UdpRoutingMessage",
                  "kind":1,
                  "startTimeUnixNano":"1729853669200512535",
                  "endTimeUnixNano":"1729853669200513345",
                  "attributes":[
                    {"key":"ockam_developer","value":{"boolValue":false}}
                  ],
                  "links":[
                    {"traceId":"3c48a8e0e4bd3c4b54c908c8329fdb51","spanId":"17168adfd2fc1679","flags":1}
                  ],
                  "status":{}
                }]
              }]
            }
          ]
        }"#;

        let mut map = HashMap::new();
        map.insert("service".to_string(), "ockam".to_string());
        map.insert("developer".to_string(), "true".to_string());
        map.insert("count".to_string(), "10".to_string());
        let expected = ResourceSpans {
            attributes: HashMap::from([
                ("service".into(), "ockam".into()),
                ("host".into(), "rendezvous".into()),
            ]),
            spans: vec![Span {
                scope: "ockam".to_string(),
                trace_id: "031dc45a859a886ca58e2b1befe31ad0".to_string(),
                span_id: "b9276e9d706ad874".to_string(),
                parent_span_id: None,
                name: "UdpRoutingMessage".to_string(),
                start_time: OffsetDateTime::from_unix_timestamp_nanos(1729853669200512535).unwrap(),
                end_time: OffsetDateTime::from_unix_timestamp_nanos(1729853669200513345).unwrap(),
                attributes: HashMap::from([("ockam_developer".into(), "false".into())]),
                links: vec![Link {
                    trace_id: "3c48a8e0e4bd3c4b54c908c8329fdb51".to_string(),
                    span_id: "17168adfd2fc1679".to_string(),
                }],
            }],
        };
        match serde_json::from_str::<ResourceSpans>(serialized) {
            Ok(deserialized) => assert_eq!(deserialized, expected),
            Err(e) => assert!(false, "deserialization error: {}", e),
        };
    }
}

trait IntoApiError<T> {
    fn into_api_error(self) -> Result<T, ApiError>;
}

impl<E: Display, T> IntoApiError<T> for Result<T, E> {
    fn into_api_error(self) -> Result<T, ApiError> {
        self.map_err(|e| ApiError::General(format!("{e}")))
    }
}
