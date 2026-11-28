PROJECT := Zoo.xcodeproj
SCHEME := Zoo
DESTINATION ?= platform=iOS Simulator,name=iPhone 18 Pro
DERIVED_DATA ?= .build
WATCH_DESTINATION ?= generic/platform=watchOS Simulator

# 引数なしの `make` は、これまでどおり `xcodegen generate`。
.DEFAULT_GOAL := generate

.PHONY: crypto-xcframework mfm-assets generate build build-watch test icon

# zoo-crypto の XCFramework と Swift のバインディング（ZooCrypto/）。xcodegen の前に要る。
crypto-xcframework:
	cd .. && $(MAKE) crypto-ffi-ios

mfm-assets:
	pnpm --dir ../frontend build:ios-mfm

generate: crypto-xcframework mfm-assets
	xcodegen generate

build:
	xcodebuild -project $(PROJECT) -scheme $(SCHEME) -configuration Debug -destination '$(DESTINATION)' -derivedDataPath '$(DERIVED_DATA)' build CODE_SIGN_IDENTITY=-

build-watch:
	xcodebuild -project $(PROJECT) -scheme ZooWatch -configuration Debug -destination '$(WATCH_DESTINATION)' -derivedDataPath '$(DERIVED_DATA)' build CODE_SIGN_IDENTITY=-

test:
	xcodebuild -project $(PROJECT) -scheme $(SCHEME) -configuration Debug -destination '$(DESTINATION)' -derivedDataPath '$(DERIVED_DATA)' -parallel-testing-enabled NO -enableCodeCoverage NO -collect-test-diagnostics never test CODE_SIGN_IDENTITY=-

icon:
	swift Scripts/GenerateIcon.swift
