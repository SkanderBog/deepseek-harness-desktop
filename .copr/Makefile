outdir ?= $(CURDIR)/.temp/copr
name ?= $(or $(basename $(filter %.spec,$(notdir $(spec)))),dsh-tauri)

srpm:
	bash $(dir $(lastword $(MAKEFILE_LIST)))srpm.sh "$(outdir)" "$(name)"

.PHONY: srpm
