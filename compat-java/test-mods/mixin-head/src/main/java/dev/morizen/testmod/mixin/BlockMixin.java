package dev.morizen.testmod.mixin;

import dev.morizen.testmod.MixinHeadMod;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(Block.class)
abstract class BlockMixin {
    @Inject(method = "defaultBlockState", at = @At("HEAD"))
    private void morizen$head(CallbackInfoReturnable<BlockState> callback) {
        MixinHeadMod.record("head");
    }
}
