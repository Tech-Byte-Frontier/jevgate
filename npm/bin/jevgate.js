#!/usr/bin/env node
"use strict";
const launcher = require("../lib/launcher.js");

const args = process.argv.slice(2);
launcher.main(args).then(
  (code) => {
    process.exitCode = code;
  },
  (error) => {
    process.exitCode = launcher.failure(args, `failed unexpectedly (${error?.message ?? error})`, process);
  },
);
