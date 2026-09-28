#!/usr/bin/env node
// Runs cargo with ~/.cargo/bin on PATH:  node tools/cargo.mjs test --workspace
import { run } from './env.mjs';

process.exit(run('cargo', process.argv.slice(2)));
