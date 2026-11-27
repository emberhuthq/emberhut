Test fixtures both apps read. `settings-entries.json` lists Settings' groups and every setting's id in order: the
desktop's descriptor list (`apps/desktop/src/store/settings/entries.rs`) and the phone's
(`apps/mobile/src/features/settings/entries.ts`) each have a test that checks it, so the two lists can't drift.
