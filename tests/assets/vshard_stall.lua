-- Test-only: stops vshard rebalancing on this instance, so the pike vshard
-- wait sees no progress. The script runs at postjoin, before picodata
-- configures the storage. At that point the public
-- vshard.storage.rebalancer_disable() would raise STORAGE_IS_DISABLED.
-- So we clear the module flag that the rebalancer loop checks, directly.
-- picodata uses the same cached vshard module, and storage configuration
-- does not reset the flag. We cannot wait for picodata to expose the module
-- instead: on some instances that takes up to 10 seconds, and the rebalancer
-- moves buckets before then.
require('vshard').storage.internal.is_rebalancer_active = false
