// SPDX-License-Identifier: GPL-2.0+
// Copyright 2017 NXP
//
// CORTINA is a registered trademark of Cortina Systems, Inc.

//! Cortina CS4340 PHY driver
//!
//! C version of this driver: [`drivers/net/phy/cortina.c`](./cortina.c)
use kernel::{
    error::code,
    net::phy::{self, DeviceId, Driver, Features},
    prelude::*,
    uapi,
};

kernel::module_phy_driver! {
    drivers: [PhyCS4340],
    device_table: [
        DeviceId::new_with_driver::<PhyCS4340>(),
    ],
    name: "cortina",
    authors: ["NXP"],
    description: "Cortina EDC CDR 10G Ethernet PHY driver",
    license: "GPL",
}

/// Global chip ID register, low word.
const VILLA_GLOBAL_CHIP_ID_LSB: u16 = 0x0;
/// Global chip ID register, high word.
const VILLA_GLOBAL_CHIP_ID_MSB: u16 = 0x1;
/// GPIO 1 interrupt status register.
const VILLA_GLOBAL_GPIO_1_INTS: u16 = 0x17;
/// Bit in `VILLA_GLOBAL_GPIO_1_INTS`: link is up when the EDC has converged.
const GPIO_1_INTS_LINK_UP: u16 = 0x8;

/// PHY identifier reported by the CS4340 chip and matched against in `probe`.
const PHY_ID_CS4340: u32 = 0x13e51002;
/// Clause-45 device address used by the CS4340 registers.
const CS4340_DEVAD: u8 = 0;

struct PhyCS4340;

/// Reads a CS4340 register with a raw clause-45 transaction on the MDIO bus.
///
/// Like `cortina_read_reg()` in the C driver: the CS4340 is reached through
/// `mdiobus_c45_read()` directly, not through phylib's MMD indirection.
fn cortina_read_reg(dev: &mut phy::Device, regnum: u16) -> Result<u16> {
    dev.c45_read_raw(CS4340_DEVAD, regnum)
}

#[vtable]
impl Driver for PhyCS4340 {
    const NAME: &'static CStr = c"Cortina CS4340";
    const PHY_DEVICE_ID: DeviceId = DeviceId::new_with_exact_mask(PHY_ID_CS4340);

    fn probe(dev: &mut phy::Device) -> Result {
        // Read device id from phy registers (the C driver turns any read
        // failure into -ENXIO).
        let id_lsb = cortina_read_reg(dev, VILLA_GLOBAL_CHIP_ID_LSB).map_err(|_| code::ENXIO)?;

        let id_msb = cortina_read_reg(dev, VILLA_GLOBAL_CHIP_ID_MSB).map_err(|_| code::ENXIO)?;

        let phy_id = (u32::from(id_lsb) << 16) | u32::from(id_msb);

        // Make sure the device tree binding matched the driver with the
        // right device.
        if phy_id != PHY_ID_CS4340 {
            // As in the C driver, report the driver name that did not match.
            dev_err!(dev, "Error matching phy with {} driver\n", PhyCS4340::NAME);
            return Err(code::ENODEV);
        }

        Ok(())
    }

    fn get_features(dev: &mut phy::Device) -> Result {
        // The C driver sets `.features = PHY_10GBIT_FEATURES` on its
        // `struct phy_driver`; `get_features` filling `supported` is the
        // equivalent the Rust vtable provides.
        dev.set_supported(Features::TenGbit);

        Ok(())
    }

    fn read_status(dev: &mut phy::Device) -> Result<u16> {
        let gpio_int_status = cortina_read_reg(dev, VILLA_GLOBAL_GPIO_1_INTS)?;

        if gpio_int_status & GPIO_1_INTS_LINK_UP != 0 {
            // up when edc_convergedS set
            dev.set_speed(uapi::SPEED_10000);
            dev.set_duplex(phy::DuplexMode::Full);
            dev.set_link(true);
        } else {
            dev.set_link(false);
        }

        Ok(0)
    }

    fn config_aneg(_dev: &mut phy::Device) -> Result {
        // The C driver uses `gen10g_config_aneg()`, which is a no-op stub in
        // `drivers/net/phy/phy-c45.c`; mirror it.
        Ok(())
    }
}
