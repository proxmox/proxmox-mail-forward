include /usr/share/dpkg/pkg-info.mk
include /usr/share/dpkg/architecture.mk

PACKAGE=proxmox-mail-forward
BUILDDIR ?= $(PACKAGE)-$(DEB_VERSION_UPSTREAM)
BUILDDIR_TMP ?= $(BUILDDIR).tmp

ifeq ($(BUILD_MODE), release)
CARGO_BUILD_ARGS += --release
COMPILEDIR := target/release
else
COMPILEDIR := target/debug
endif

CARGO ?= cargo

DEB=$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
DBG_DEB=$(PACKAGE)-dbgsym_$(DEB_VERSION_UPSTREAM_REVISION)_$(DEB_BUILD_ARCH).deb
DSC=rust-$(PACKAGE)_$(DEB_VERSION_UPSTREAM_REVISION).dsc

DEBS=$(DEB) $(DBG_DEB)

.PHONY: build
build:
	@echo "Setting Cargo.toml version to: $(DEB_VERSION_UPSTREAM)"
	sed -i -e 's/^version =.*$$/version = "$(DEB_VERSION_UPSTREAM)"/' Cargo.toml
	rm -rf $(BUILDDIR) $(BUILDDIR_TMP); mkdir $(BUILDDIR_TMP)
	cp -a debian \
	  Cargo.toml src \
	  Makefile \
	  $(BUILDDIR_TMP)
	rm -f $(BUILDDIR_TMP)/Cargo.lock
	find $(BUILDDIR_TMP)/debian -name "*.hint" -delete
	mv $(BUILDDIR_TMP) $(BUILDDIR)

.PHONY: deb
$(DEBS): deb
deb: build
	cd $(BUILDDIR); dpkg-buildpackage -b -us -uc --no-pre-clean
	lintian $(DEBS)

.PHONY: dsc
dsc: $(DSC)
$(DSC): build
	cd $(BUILDDIR); dpkg-buildpackage -S -us -uc -d -nc
	lintian $(DSC)

.PHONY: dinstall
dinstall: $(DEBS)
	dpkg -i $(DEBS)

.PHONY: cargo-build
cargo-build:
	$(CARGO) build $(CARGO_BUILD_ARGS) \
	    --package proxmox-mail-forward \
	    --bin proxmox-mail-forward

install: cargo-build
	install -dm755 $(DESTDIR)/usr/bin
	install -m4755 -o root -g root $(COMPILEDIR)/proxmox-mail-forward $(DESTDIR)/usr/bin/proxmox-mail-forward

.PHONY: upload
upload: $(DEBS)
	tar cf - $(DEBS) | ssh -X repoman@repo.proxmox.com -- upload --product "pve,pbs" --dist bullseye --arch $(DEB_BUILD_ARCH)

.PHONY: distclean
distclean: clean

.PHONY: clean
clean:
	cargo clean
	rm -rf *.deb *.buildinfo *.changes *.dsc rust-$(PACKAGE)_*.tar.?z $(BUILDDIR) $(BUILDDIR_TMP)
	find . -name '*~' -exec rm {} ';'
