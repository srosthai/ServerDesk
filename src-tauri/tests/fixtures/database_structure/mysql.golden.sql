-- create_table users
CREATE TABLE `app db`.`Users` (
  `ID` bigint unsigned NOT NULL AUTO_INCREMENT,
  `Slug Value` varchar(180) GENERATED ALWAYS AS (lower(`Email Address`)) STORED NULL,
  `Email Address` varchar(320) NOT NULL DEFAULT 'n/a' COMMENT 'Login address',
  PRIMARY KEY (`ID`)
);

-- rename_table users to accounts
RENAME TABLE `app db`.`Users` TO `app db`.`Accounts`;

-- add_column display_name
ALTER TABLE `app db`.`Users` ADD COLUMN `Display Name` varchar(120) NOT NULL DEFAULT 'Robert''; DROP TABLE users; --' AFTER `Email Address`;

-- alter_column email
ALTER TABLE `app db`.`Users` MODIFY COLUMN `Email Address` varchar(500) NOT NULL DEFAULT NULL COMMENT 'Primary address';

-- rename_column email
ALTER TABLE `app db`.`Users` RENAME COLUMN `Email Address` TO `Email`;

-- reorder_column id first
ALTER TABLE `app db`.`Users` MODIFY COLUMN `ID` bigint unsigned NOT NULL FIRST;

-- create_index users_email
CREATE UNIQUE INDEX `Users Email Index` USING BTREE ON `app db`.`Users` (`Email`);

-- rename_index users_email
ALTER TABLE `app db`.`Users` RENAME INDEX `Users Email Index` TO `users_email_idx`;

-- add_foreign_key orders_user
ALTER TABLE `app db`.`Orders` ADD CONSTRAINT `orders_user_fk` FOREIGN KEY (`User ID`) REFERENCES `app db`.`Users` (`ID`) ON DELETE CASCADE ON UPDATE NO ACTION;

-- drop_foreign_key orders_user
ALTER TABLE `app db`.`Orders` DROP FOREIGN KEY `orders_user_fk`;

-- drop_table accounts
DROP TABLE `app db`.`Accounts`;

