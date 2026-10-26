PROJECT := Zoo.xcodeproj
SCHEME := Zoo
DESTINATION ?= platform=iOS Simulator,name=iPhone 18 Pro
DERIVED_DATA ?= .build

.PHONY: generate build test icon

generate:
	xcodegen generate

build:
	xcodebuild -project $(PROJECT) -scheme $(SCHEME) -configuration Debug -destination '$(DESTINATION)' -derivedDataPath '$(DERIVED_DATA)' build CODE_SIGN_IDENTITY=-

test:
	xcodebuild -project $(PROJECT) -scheme $(SCHEME) -configuration Debug -destination '$(DESTINATION)' -derivedDataPath '$(DERIVED_DATA)' -parallel-testing-enabled NO -enableCodeCoverage NO -collect-test-diagnostics never test CODE_SIGN_IDENTITY=-

icon:
	swift Scripts/GenerateIcon.swift
