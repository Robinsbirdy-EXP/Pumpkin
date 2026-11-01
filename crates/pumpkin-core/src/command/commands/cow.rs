use std::time::Instant;

use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::color::NamedColor;

use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};

const DESCRIPTION: &str = "Manage Copy-on-Write (CoW) templates and world instances.";
const PERMISSION: &str = "pumpkin:command.cow";

struct LoadTemplateExecutor;

impl CommandExecutor for LoadTemplateExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let template_name = StringArgumentType::get(context, "template")?.to_string();
        let server = context.server();
        let start = Instant::now();

        match server.cow_load_template(&template_name) {
            Ok(()) => {
                let chunks = pumpkin_cow_world::TEMPLATES
                    .get(&template_name)
                    .map_or(0, |tpl| tpl.chunks.len());
                let elapsed = start.elapsed();
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Loaded template '{template_name}' ({chunks} chunks) in {elapsed:.2?}"
                    ))
                    .color_named(NamedColor::Green),
                    true,
                );
                Ok(1)
            }
            Err(err) => {
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Failed to load template '{template_name}': {err}"
                    ))
                    .color_named(NamedColor::Red),
                    false,
                );
                Ok(0)
            }
        }
    }
}

struct SpawnInstanceExecutor;

impl CommandExecutor for SpawnInstanceExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let template_name = StringArgumentType::get(context, "template")?.to_string();
        let instance_id = StringArgumentType::get(context, "instance")?.to_string();
        let server = context.server();
        let start = Instant::now();

        match server.cow_spawn_instance(&template_name, &instance_id) {
            Ok(_) => {
                let elapsed = start.elapsed();
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Spawned instance '{instance_id}' from template '{template_name}' in {elapsed:.2?}"
                    ))
                    .color_named(NamedColor::Green),
                    true,
                );
                Ok(1)
            }
            Err(err) => {
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Failed to spawn instance '{instance_id}': {err}"
                    ))
                    .color_named(NamedColor::Red),
                    false,
                );
                Ok(0)
            }
        }
    }
}

struct ResetInstanceExecutor;

impl CommandExecutor for ResetInstanceExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let instance_id = StringArgumentType::get(context, "instance")?.to_string();
        let server = context.server();
        let start = Instant::now();

        match server.cow_reset_instance(&instance_id) {
            Ok(reset_chunks) => {
                let elapsed = start.elapsed();
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Reset instance '{instance_id}' ({reset_chunks} dirty chunks restored) in {elapsed:.2?}"
                    ))
                    .color_named(NamedColor::Green),
                    true,
                );
                Ok(reset_chunks as i32)
            }
            Err(err) => {
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Failed to reset instance '{instance_id}': {err}"
                    ))
                    .color_named(NamedColor::Red),
                    false,
                );
                Ok(0)
            }
        }
    }
}

struct DestroyInstanceExecutor;

impl CommandExecutor for DestroyInstanceExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let instance_id = StringArgumentType::get(context, "instance")?.to_string();
        let server = context.server();

        match server.cow_destroy_instance(&instance_id) {
            Ok(()) => {
                context.source.send_feedback(
                    TextComponent::text(format!("[CoW] Destroyed instance '{instance_id}'"))
                        .color_named(NamedColor::Green),
                    true,
                );
                Ok(1)
            }
            Err(err) => {
                context.source.send_feedback(
                    TextComponent::text(format!(
                        "[CoW] Failed to destroy instance '{instance_id}': {err}"
                    ))
                    .color_named(NamedColor::Red),
                    false,
                );
                Ok(0)
            }
        }
    }
}

struct StatsExecutor;

impl CommandExecutor for StatsExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let templates_count = pumpkin_cow_world::TEMPLATES.len();
        let instances_count = pumpkin_cow_world::INSTANCES.len();

        let mut summary =
            format!("[CoW] Templates: {templates_count}, Active Instances: {instances_count}");

        for entry in pumpkin_cow_world::TEMPLATES.iter() {
            summary.push_str(&format!(
                "\n  Template '{}': {} chunks",
                entry.key(),
                entry.value().chunks.len()
            ));
        }

        for entry in pumpkin_cow_world::INSTANCES.iter() {
            let (total_chunks, dirty_chunks) = entry.value().stats();
            summary.push_str(&format!(
                "\n  Instance '{}' (from '{}'): {total_chunks} chunks ({dirty_chunks} dirty)",
                entry.key(),
                entry.value().template_name
            ));
        }

        context.source.send_feedback(
            TextComponent::text(summary).color_named(NamedColor::Aqua),
            false,
        );
        Ok(1)
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("cow", DESCRIPTION)
            .requires(PERMISSION)
            .then(literal("load").then(
                argument("template", StringArgumentType::SingleWord).executes(LoadTemplateExecutor),
            ))
            .then(
                literal("spawn").then(
                    argument("template", StringArgumentType::SingleWord).then(
                        argument("instance", StringArgumentType::SingleWord)
                            .executes(SpawnInstanceExecutor),
                    ),
                ),
            )
            .then(
                literal("reset").then(
                    argument("instance", StringArgumentType::SingleWord)
                        .executes(ResetInstanceExecutor),
                ),
            )
            .then(
                literal("destroy").then(
                    argument("instance", StringArgumentType::SingleWord)
                        .executes(DestroyInstanceExecutor),
                ),
            )
            .then(literal("stats").executes(StatsExecutor)),
    );
}
