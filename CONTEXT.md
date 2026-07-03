# Herdr Layout

Herdr Layout defines small, repeatable workspace tab setups for Herdr users.

## Language

**Layout**:
One of three nameless slots a user can trigger through fixed Herdr plugin actions. A layout contains tab targets and owns whether its first target may claim the current idle tab.
_Avoid_: Recipe, profile, preset

**Tab Target**:
A desired Herdr tab identified by label and shell-line command. Applying a layout reuses or creates tabs until each target exists and is running.
_Avoid_: Tab config, pane command

**Repo Override**:
A nearest-ancestor `.herdr-layout.yaml` or `.herdr-layout.yml` file that can override individual global layout slots for that working directory. Slots missing from the repo override fall back to the global layout slot.
_Avoid_: Project config, local preset
