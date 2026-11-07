//! 两项静态 dbusmenu；布局固定深度一，不保存客户端查询参数或事件队列。
use super::{TrayEvent, TrayEvents};
use ashpd::{
    zbus::{self, fdo},
    zvariant::{OwnedValue, Str, Structure},
};
use std::{collections::HashMap, sync::Arc};

pub(super) const PATH: &str = "/Menu";
type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);
pub(super) struct TrayMenu(pub(super) Arc<TrayEvents>);

fn valid_id(id: i32) -> fdo::Result<()> {
    if (0..=2).contains(&id) {
        Ok(())
    } else {
        Err(fdo::Error::InvalidArgs("Unknown menu item".into()))
    }
}

fn properties(id: i32, names: &[String], locale: crate::ui::toolbar_i18n::ToolbarLocale) -> fdo::Result<Properties> {
    valid_id(id)?;
    if names.len() > 16 || names.iter().any(|name| name.len() > 128) {
        return Err(fdo::Error::InvalidArgs("Too many property names".into()));
    }
    let mut props = HashMap::new();
    if id == 0 {
        props.insert(
            "children-display".into(),
            OwnedValue::from(Str::from("submenu")),
        );
    } else {
        props.insert(
            "label".into(),
            OwnedValue::from(Str::from(if id == 1 {
                locale.app_text("tray.open")
            } else {
                locale.app_text("tray.quit")
            })),
        );
        props.insert("enabled".into(), OwnedValue::from(true));
        props.insert("visible".into(), OwnedValue::from(true));
    }
    if !names.is_empty() {
        props.retain(|key, _| names.iter().any(|name| name == key));
    }
    Ok(props)
}

#[zbus::interface(name = "com.canonical.dbusmenu", crate = "ashpd::zbus", spawn = false)]
impl TrayMenu {
    #[zbus(signal)]
    async fn layout_updated(emitter: &zbus::object_server::SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        4
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(out_args("revision", "layout"))]
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> fdo::Result<(u32, Layout)> {
        let locale = self.0.locale();
        let props = properties(parent_id, &property_names, locale)?;
        let mut children = Vec::new();
        if parent_id == 0 && recursion_depth != 0 {
            for id in [1, 2] {
                let child: Layout = (id, properties(id, &property_names, locale)?, Vec::new());
                children.push(
                    OwnedValue::try_from(Structure::from(child))
                        .map_err(|error| fdo::Error::Failed(error.to_string()))?,
                );
            }
        }
        Ok((self.0.menu_revision(), (parent_id, props, children)))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> fdo::Result<Vec<(i32, Properties)>> {
        if ids.len() > 64 {
            return Err(fdo::Error::InvalidArgs("Too many menu items".into()));
        }
        let ids = if ids.is_empty() { vec![0, 1, 2] } else { ids };
        let locale = self.0.locale();
        ids.into_iter()
            .map(|id| Ok((id, properties(id, &property_names, locale)?)))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedValue> {
        properties(id, &[], self.0.locale())?
            .remove(&name)
            .ok_or_else(|| fdo::Error::InvalidArgs("Unknown menu property".into()))
    }

    fn event(
        &self,
        id: i32,
        event_id: &str,
        _data: OwnedValue,
        _timestamp: u32,
    ) -> fdo::Result<()> {
        valid_id(id)?;
        if event_id == "clicked" {
            match id {
                1 => self.0.send(TrayEvent::Restore),
                2 => self.0.send(TrayEvent::Quit),
                _ => {}
            }
        }
        Ok(())
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> fdo::Result<Vec<i32>> {
        if events.len() > 64 {
            return Err(fdo::Error::InvalidArgs("Too many menu events".into()));
        }
        let mut failed = Vec::new();
        for (id, event, data, timestamp) in events {
            if self.event(id, &event, data, timestamp).is_err() {
                failed.push(id);
            }
        }
        Ok(failed)
    }

    fn about_to_show(&self, id: i32) -> fdo::Result<bool> {
        valid_id(id)?;
        Ok(false)
    }

    #[zbus(out_args("updates_needed", "id_errors"))]
    fn about_to_show_group(&self, ids: Vec<i32>) -> fdo::Result<(Vec<i32>, Vec<i32>)> {
        if ids.len() > 64 {
            return Err(fdo::Error::InvalidArgs("Too many menu items".into()));
        }
        Ok((
            Vec::new(),
            ids.into_iter()
                .filter(|id| valid_id(*id).is_err())
                .collect(),
        ))
    }
}
